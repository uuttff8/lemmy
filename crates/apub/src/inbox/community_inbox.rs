use crate::{
  activities::receive::verify_activity_domains_valid,
  inbox::{
    assert_activity_not_local,
    get_activity_id,
    get_activity_to_and_cc,
    inbox_verify_http_signature,
    is_activity_already_known,
    is_addressed_to_public,
    receive_for_community::{
      receive_create_for_community,
      receive_delete_for_community,
      receive_dislike_for_community,
      receive_like_for_community,
      receive_undo_for_community,
      receive_update_for_community,
    },
  },
  insert_activity,
  ActorType,
};
use activitystreams::{
  activity::{kind::FollowType, ActorAndObject, Follow, Undo},
  base::AnyBase,
  prelude::*,
};
use actix_web::{web, HttpRequest, HttpResponse};
use anyhow::{anyhow, Context};
use lemmy_api_structs::blocking;
use lemmy_db_queries::{source::community::Community_, ApubObject, DbPool, Followable};
use lemmy_db_schema::source::{
  community::{Community, CommunityFollower, CommunityFollowerForm},
  person::Person,
};
use lemmy_db_views_actor::community_person_ban_view::CommunityPersonBanView;
use lemmy_utils::{location_info, LemmyError};
use lemmy_websocket::LemmyContext;
use log::info;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use url::Url;

/// Allowed activities for community inbox.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum CommunityValidTypes {
  Follow,  // follow request from a person
  Undo,    // unfollow from a person
  Create,  // create post or comment
  Update,  // update post or comment
  Like,    // upvote post or comment
  Dislike, // downvote post or comment
  Delete,  // post or comment deleted by creator
  Remove,  // post or comment removed by mod or admin
}

pub type CommunityAcceptedActivities = ActorAndObject<CommunityValidTypes>;

/// Handler for all incoming receive to community inboxes.
pub async fn community_inbox(
  request: HttpRequest,
  input: web::Json<CommunityAcceptedActivities>,
  path: web::Path<String>,
  context: web::Data<LemmyContext>,
) -> Result<HttpResponse, LemmyError> {
  let activity = input.into_inner();
  // First of all check the http signature
  let request_counter = &mut 0;
  let actor = inbox_verify_http_signature(&activity, &context, request, request_counter).await?;

  // Do nothing if we received the same activity before
  let activity_id = get_activity_id(&activity, &actor.actor_id())?;
  if is_activity_already_known(context.pool(), &activity_id).await? {
    return Ok(HttpResponse::Ok().finish());
  }

  // Check if the activity is actually meant for us
  let path = path.into_inner();
  let community = blocking(&context.pool(), move |conn| {
    Community::read_from_name(&conn, &path)
  })
  .await??;
  let to_and_cc = get_activity_to_and_cc(&activity);
  if !to_and_cc.contains(&&community.actor_id()) {
    return Err(anyhow!("Activity delivered to wrong community").into());
  }

  assert_activity_not_local(&activity)?;
  insert_activity(&activity_id, activity.clone(), false, true, context.pool()).await?;

  info!(
    "Community {} received activity {:?} from {}",
    community.name,
    &activity.id_unchecked(),
    &actor.actor_id()
  );

  community_receive_message(
    activity.clone(),
    community.clone(),
    actor.as_ref(),
    &context,
    request_counter,
  )
  .await
}

/// Receives Follow, Undo/Follow, post actions, comment actions (including votes)
pub(crate) async fn community_receive_message(
  activity: CommunityAcceptedActivities,
  to_community: Community,
  actor: &dyn ActorType,
  context: &LemmyContext,
  request_counter: &mut i32,
) -> Result<HttpResponse, LemmyError> {
  // Only persons can send activities to the community, so we can get the actor as person
  // unconditionally.
  let actor_id = actor.actor_id();
  let person = blocking(&context.pool(), move |conn| {
    Person::read_from_apub_id(&conn, &actor_id.into())
  })
  .await??;
  check_community_or_site_ban(&person, to_community.id, context.pool()).await?;

  let any_base = activity.clone().into_any_base()?;
  let actor_url = actor.actor_id();
  let activity_kind = activity.kind().context(location_info!())?;
  let do_announce = match activity_kind {
    CommunityValidTypes::Follow => {
      handle_follow(any_base.clone(), person, &to_community, &context).await?;
      false
    }
    CommunityValidTypes::Undo => {
      handle_undo(
        context,
        activity.clone(),
        actor_url,
        &to_community,
        request_counter,
      )
      .await?
    }
    CommunityValidTypes::Create => {
      receive_create_for_community(context, any_base.clone(), &actor_url, request_counter).await?;
      true
    }
    CommunityValidTypes::Update => {
      receive_update_for_community(context, any_base.clone(), &actor_url, request_counter).await?;
      true
    }
    CommunityValidTypes::Like => {
      receive_like_for_community(context, any_base.clone(), &actor_url, request_counter).await?;
      true
    }
    CommunityValidTypes::Dislike => {
      receive_dislike_for_community(context, any_base.clone(), &actor_url, request_counter).await?;
      true
    }
    CommunityValidTypes::Delete => {
      receive_delete_for_community(context, any_base.clone(), &actor_url).await?;
      true
    }
    CommunityValidTypes::Remove => {
      // TODO: we dont support remote mods, so this is ignored for now
      //receive_remove_for_community(context, any_base.clone(), &person_url).await?
      false
    }
  };

  if do_announce {
    // Check again that the activity is public, just to be sure
    is_addressed_to_public(&activity)?;
    to_community
      .send_announce(activity.into_any_base()?, context)
      .await?;
  }

  Ok(HttpResponse::Ok().finish())
}

/// Handle a follow request from a remote person, adding the person as follower and returning an
/// Accept activity.
async fn handle_follow(
  activity: AnyBase,
  person: Person,
  community: &Community,
  context: &LemmyContext,
) -> Result<HttpResponse, LemmyError> {
  let follow = Follow::from_any_base(activity)?.context(location_info!())?;
  verify_activity_domains_valid(&follow, &person.actor_id(), false)?;

  let community_follower_form = CommunityFollowerForm {
    community_id: community.id,
    person_id: person.id,
    pending: false,
  };

  // This will fail if they're already a follower, but ignore the error.
  blocking(&context.pool(), move |conn| {
    CommunityFollower::follow(&conn, &community_follower_form).ok()
  })
  .await?;

  community.send_accept_follow(follow, context).await?;

  Ok(HttpResponse::Ok().finish())
}

async fn handle_undo(
  context: &LemmyContext,
  activity: CommunityAcceptedActivities,
  actor_url: Url,
  to_community: &Community,
  request_counter: &mut i32,
) -> Result<bool, LemmyError> {
  let inner_kind = activity
    .object()
    .is_single_kind(&FollowType::Follow.to_string());
  let any_base = activity.into_any_base()?;
  if inner_kind {
    handle_undo_follow(any_base, actor_url, to_community, &context).await?;
    Ok(false)
  } else {
    receive_undo_for_community(context, any_base, &actor_url, request_counter).await?;
    Ok(true)
  }
}

/// Handle `Undo/Follow` from a person, removing the person from followers list.
async fn handle_undo_follow(
  activity: AnyBase,
  person_url: Url,
  community: &Community,
  context: &LemmyContext,
) -> Result<(), LemmyError> {
  let undo = Undo::from_any_base(activity)?.context(location_info!())?;
  verify_activity_domains_valid(&undo, &person_url, true)?;

  let object = undo.object().to_owned().one().context(location_info!())?;
  let follow = Follow::from_any_base(object)?.context(location_info!())?;
  verify_activity_domains_valid(&follow, &person_url, false)?;

  let person = blocking(&context.pool(), move |conn| {
    Person::read_from_apub_id(&conn, &person_url.into())
  })
  .await??;
  let community_follower_form = CommunityFollowerForm {
    community_id: community.id,
    person_id: person.id,
    pending: false,
  };

  // This will fail if they aren't a follower, but ignore the error.
  blocking(&context.pool(), move |conn| {
    CommunityFollower::unfollow(&conn, &community_follower_form).ok()
  })
  .await?;

  Ok(())
}

pub(crate) async fn check_community_or_site_ban(
  person: &Person,
  community_id: i32,
  pool: &DbPool,
) -> Result<(), LemmyError> {
  if person.banned {
    return Err(anyhow!("Person is banned from site").into());
  }
  let person_id = person.id;
  let is_banned =
    move |conn: &'_ _| CommunityPersonBanView::get(conn, person_id, community_id).is_ok();
  if blocking(pool, is_banned).await? {
    return Err(anyhow!("Person is banned from community").into());
  }

  Ok(())
}
