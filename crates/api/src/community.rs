use crate::{
  check_community_ban,
  get_local_user_view_from_jwt,
  get_local_user_view_from_jwt_opt,
  is_admin,
  is_mod_or_admin,
  Perform,
};
use actix_web::web::Data;
use anyhow::Context;
use lemmy_api_structs::{blocking, community::*};
use lemmy_apub::{
  generate_apub_endpoint,
  generate_followers_url,
  generate_inbox_url,
  generate_shared_inbox_url,
  ActorType,
  EndpointType,
};
use lemmy_db_queries::{
  diesel_option_overwrite_to_url,
  source::{
    comment::Comment_,
    community::{CommunityModerator_, Community_},
    post::Post_,
  },
  ApubObject,
  Bannable,
  Crud,
  Followable,
  Joinable,
  ListingType,
  SortType,
};
use lemmy_db_schema::{
  naive_now,
  source::{comment::Comment, community::*, moderator::*, post::Post, site::*},
};
use lemmy_db_views::comment_view::CommentQueryBuilder;
use lemmy_db_views_actor::{
  community_follower_view::CommunityFollowerView,
  community_moderator_view::CommunityModeratorView,
  community_view::{CommunityQueryBuilder, CommunityView},
  person_view::PersonViewSafe,
};
use lemmy_utils::{
  apub::generate_actor_keypair,
  location_info,
  utils::{check_slurs, check_slurs_opt, is_valid_community_name, naive_from_unix},
  ApiError,
  ConnectionId,
  LemmyError,
};
use lemmy_websocket::{
  messages::{GetCommunityUsersOnline, SendCommunityRoomMessage},
  LemmyContext,
  UserOperation,
};
use std::str::FromStr;

#[async_trait::async_trait(?Send)]
impl Perform for GetCommunity {
  type Response = GetCommunityResponse;

  async fn perform(
    &self,
    context: &Data<LemmyContext>,
    _websocket_id: Option<ConnectionId>,
  ) -> Result<GetCommunityResponse, LemmyError> {
    let data: &GetCommunity = &self;
    let local_user_view = get_local_user_view_from_jwt_opt(&data.auth, context.pool()).await?;
    let person_id = local_user_view.map(|u| u.person.id);

    let community_id = match data.id {
      Some(id) => id,
      None => {
        let name = data.name.to_owned().unwrap_or_else(|| "main".to_string());
        match blocking(context.pool(), move |conn| {
          Community::read_from_name(conn, &name)
        })
        .await?
        {
          Ok(community) => community,
          Err(_e) => return Err(ApiError::err("couldnt_find_community").into()),
        }
        .id
      }
    };

    let community_view = match blocking(context.pool(), move |conn| {
      CommunityView::read(conn, community_id, person_id)
    })
    .await?
    {
      Ok(community) => community,
      Err(_e) => return Err(ApiError::err("couldnt_find_community").into()),
    };

    let moderators: Vec<CommunityModeratorView> = match blocking(context.pool(), move |conn| {
      CommunityModeratorView::for_community(conn, community_id)
    })
    .await?
    {
      Ok(moderators) => moderators,
      Err(_e) => return Err(ApiError::err("couldnt_find_community").into()),
    };

    let online = context
      .chat_server()
      .send(GetCommunityUsersOnline { community_id })
      .await
      .unwrap_or(1);

    let res = GetCommunityResponse {
      community_view,
      moderators,
      online,
    };

    // Return the jwt
    Ok(res)
  }
}

#[async_trait::async_trait(?Send)]
impl Perform for CreateCommunity {
  type Response = CommunityResponse;

  async fn perform(
    &self,
    context: &Data<LemmyContext>,
    _websocket_id: Option<ConnectionId>,
  ) -> Result<CommunityResponse, LemmyError> {
    let data: &CreateCommunity = &self;
    let local_user_view = get_local_user_view_from_jwt(&data.auth, context.pool()).await?;

    check_slurs(&data.name)?;
    check_slurs(&data.title)?;
    check_slurs_opt(&data.description)?;

    if !is_valid_community_name(&data.name) {
      return Err(ApiError::err("invalid_community_name").into());
    }

    // Double check for duplicate community actor_ids
    let community_actor_id = generate_apub_endpoint(EndpointType::Community, &data.name)?;
    let actor_id_cloned = community_actor_id.to_owned();
    let community_dupe = blocking(context.pool(), move |conn| {
      Community::read_from_apub_id(conn, &actor_id_cloned)
    })
    .await?;
    if community_dupe.is_ok() {
      return Err(ApiError::err("community_already_exists").into());
    }

    // Check to make sure the icon and banners are urls
    let icon = diesel_option_overwrite_to_url(&data.icon)?;
    let banner = diesel_option_overwrite_to_url(&data.banner)?;

    // When you create a community, make sure the user becomes a moderator and a follower
    let keypair = generate_actor_keypair()?;

    let community_form = CommunityForm {
      name: data.name.to_owned(),
      title: data.title.to_owned(),
      description: data.description.to_owned(),
      icon,
      banner,
      creator_id: local_user_view.person.id,
      removed: None,
      deleted: None,
      nsfw: data.nsfw,
      updated: None,
      actor_id: Some(community_actor_id.to_owned()),
      local: true,
      private_key: Some(keypair.private_key),
      public_key: Some(keypair.public_key),
      last_refreshed_at: None,
      published: None,
      followers_url: Some(generate_followers_url(&community_actor_id)?),
      inbox_url: Some(generate_inbox_url(&community_actor_id)?),
      shared_inbox_url: Some(Some(generate_shared_inbox_url(&community_actor_id)?)),
    };

    let inserted_community = match blocking(context.pool(), move |conn| {
      Community::create(conn, &community_form)
    })
    .await?
    {
      Ok(community) => community,
      Err(_e) => return Err(ApiError::err("community_already_exists").into()),
    };

    // The community creator becomes a moderator
    let community_moderator_form = CommunityModeratorForm {
      community_id: inserted_community.id,
      person_id: local_user_view.person.id,
    };

    let join = move |conn: &'_ _| CommunityModerator::join(conn, &community_moderator_form);
    if blocking(context.pool(), join).await?.is_err() {
      return Err(ApiError::err("community_moderator_already_exists").into());
    }

    // Follow your own community
    let community_follower_form = CommunityFollowerForm {
      community_id: inserted_community.id,
      person_id: local_user_view.person.id,
      pending: false,
    };

    let follow = move |conn: &'_ _| CommunityFollower::follow(conn, &community_follower_form);
    if blocking(context.pool(), follow).await?.is_err() {
      return Err(ApiError::err("community_follower_already_exists").into());
    }

    let person_id = local_user_view.person.id;
    let community_view = blocking(context.pool(), move |conn| {
      CommunityView::read(conn, inserted_community.id, Some(person_id))
    })
    .await??;

    Ok(CommunityResponse { community_view })
  }
}

#[async_trait::async_trait(?Send)]
impl Perform for EditCommunity {
  type Response = CommunityResponse;

  async fn perform(
    &self,
    context: &Data<LemmyContext>,
    websocket_id: Option<ConnectionId>,
  ) -> Result<CommunityResponse, LemmyError> {
    let data: &EditCommunity = &self;
    let local_user_view = get_local_user_view_from_jwt(&data.auth, context.pool()).await?;

    check_slurs(&data.title)?;
    check_slurs_opt(&data.description)?;

    // Verify its a mod (only mods can edit it)
    let community_id = data.community_id;
    let mods: Vec<i32> = blocking(context.pool(), move |conn| {
      CommunityModeratorView::for_community(conn, community_id)
        .map(|v| v.into_iter().map(|m| m.moderator.id).collect())
    })
    .await??;
    if !mods.contains(&local_user_view.person.id) {
      return Err(ApiError::err("not_a_moderator").into());
    }

    let community_id = data.community_id;
    let read_community = blocking(context.pool(), move |conn| {
      Community::read(conn, community_id)
    })
    .await??;

    let icon = diesel_option_overwrite_to_url(&data.icon)?;
    let banner = diesel_option_overwrite_to_url(&data.banner)?;

    let community_form = CommunityForm {
      name: read_community.name,
      title: data.title.to_owned(),
      description: data.description.to_owned(),
      icon,
      banner,
      creator_id: read_community.creator_id,
      removed: Some(read_community.removed),
      deleted: Some(read_community.deleted),
      nsfw: data.nsfw,
      updated: Some(naive_now()),
      actor_id: Some(read_community.actor_id),
      local: read_community.local,
      private_key: read_community.private_key,
      public_key: read_community.public_key,
      last_refreshed_at: None,
      published: None,
      followers_url: None,
      inbox_url: None,
      shared_inbox_url: None,
    };

    let community_id = data.community_id;
    match blocking(context.pool(), move |conn| {
      Community::update(conn, community_id, &community_form)
    })
    .await?
    {
      Ok(community) => community,
      Err(_e) => return Err(ApiError::err("couldnt_update_community").into()),
    };

    // TODO there needs to be some kind of an apub update
    // process for communities and users

    let community_id = data.community_id;
    let person_id = local_user_view.person.id;
    let community_view = blocking(context.pool(), move |conn| {
      CommunityView::read(conn, community_id, Some(person_id))
    })
    .await??;

    let res = CommunityResponse { community_view };

    send_community_websocket(&res, context, websocket_id, UserOperation::EditCommunity);

    Ok(res)
  }
}

#[async_trait::async_trait(?Send)]
impl Perform for DeleteCommunity {
  type Response = CommunityResponse;

  async fn perform(
    &self,
    context: &Data<LemmyContext>,
    websocket_id: Option<ConnectionId>,
  ) -> Result<CommunityResponse, LemmyError> {
    let data: &DeleteCommunity = &self;
    let local_user_view = get_local_user_view_from_jwt(&data.auth, context.pool()).await?;

    // Verify its the creator (only a creator can delete the community)
    let community_id = data.community_id;
    let read_community = blocking(context.pool(), move |conn| {
      Community::read(conn, community_id)
    })
    .await??;
    if read_community.creator_id != local_user_view.person.id {
      return Err(ApiError::err("no_community_edit_allowed").into());
    }

    // Do the delete
    let community_id = data.community_id;
    let deleted = data.deleted;
    let updated_community = match blocking(context.pool(), move |conn| {
      Community::update_deleted(conn, community_id, deleted)
    })
    .await?
    {
      Ok(community) => community,
      Err(_e) => return Err(ApiError::err("couldnt_update_community").into()),
    };

    // Send apub messages
    if deleted {
      updated_community.send_delete(context).await?;
    } else {
      updated_community.send_undo_delete(context).await?;
    }

    let community_id = data.community_id;
    let person_id = local_user_view.person.id;
    let community_view = blocking(context.pool(), move |conn| {
      CommunityView::read(conn, community_id, Some(person_id))
    })
    .await??;

    let res = CommunityResponse { community_view };

    send_community_websocket(&res, context, websocket_id, UserOperation::DeleteCommunity);

    Ok(res)
  }
}

#[async_trait::async_trait(?Send)]
impl Perform for RemoveCommunity {
  type Response = CommunityResponse;

  async fn perform(
    &self,
    context: &Data<LemmyContext>,
    websocket_id: Option<ConnectionId>,
  ) -> Result<CommunityResponse, LemmyError> {
    let data: &RemoveCommunity = &self;
    let local_user_view = get_local_user_view_from_jwt(&data.auth, context.pool()).await?;

    // Verify its an admin (only an admin can remove a community)
    is_admin(&local_user_view)?;

    // Do the remove
    let community_id = data.community_id;
    let removed = data.removed;
    let updated_community = match blocking(context.pool(), move |conn| {
      Community::update_removed(conn, community_id, removed)
    })
    .await?
    {
      Ok(community) => community,
      Err(_e) => return Err(ApiError::err("couldnt_update_community").into()),
    };

    // Mod tables
    let expires = match data.expires {
      Some(time) => Some(naive_from_unix(time)),
      None => None,
    };
    let form = ModRemoveCommunityForm {
      mod_person_id: local_user_view.person.id,
      community_id: data.community_id,
      removed: Some(removed),
      reason: data.reason.to_owned(),
      expires,
    };
    blocking(context.pool(), move |conn| {
      ModRemoveCommunity::create(conn, &form)
    })
    .await??;

    // Apub messages
    if removed {
      updated_community.send_remove(context).await?;
    } else {
      updated_community.send_undo_remove(context).await?;
    }

    let community_id = data.community_id;
    let person_id = local_user_view.person.id;
    let community_view = blocking(context.pool(), move |conn| {
      CommunityView::read(conn, community_id, Some(person_id))
    })
    .await??;

    let res = CommunityResponse { community_view };

    send_community_websocket(&res, context, websocket_id, UserOperation::RemoveCommunity);

    Ok(res)
  }
}

#[async_trait::async_trait(?Send)]
impl Perform for ListCommunities {
  type Response = ListCommunitiesResponse;

  async fn perform(
    &self,
    context: &Data<LemmyContext>,
    _websocket_id: Option<ConnectionId>,
  ) -> Result<ListCommunitiesResponse, LemmyError> {
    let data: &ListCommunities = &self;
    let local_user_view = get_local_user_view_from_jwt_opt(&data.auth, context.pool()).await?;

    let person_id = match &local_user_view {
      Some(uv) => Some(uv.person.id),
      None => None,
    };

    // Don't show NSFW by default
    let show_nsfw = match &local_user_view {
      Some(uv) => uv.local_user.show_nsfw,
      None => false,
    };

    let type_ = ListingType::from_str(&data.type_)?;
    let sort = SortType::from_str(&data.sort)?;

    let page = data.page;
    let limit = data.limit;
    let communities = blocking(context.pool(), move |conn| {
      CommunityQueryBuilder::create(conn)
        .listing_type(&type_)
        .sort(&sort)
        .show_nsfw(show_nsfw)
        .my_person_id(person_id)
        .page(page)
        .limit(limit)
        .list()
    })
    .await??;

    // Return the jwt
    Ok(ListCommunitiesResponse { communities })
  }
}

#[async_trait::async_trait(?Send)]
impl Perform for FollowCommunity {
  type Response = CommunityResponse;

  async fn perform(
    &self,
    context: &Data<LemmyContext>,
    _websocket_id: Option<ConnectionId>,
  ) -> Result<CommunityResponse, LemmyError> {
    let data: &FollowCommunity = &self;
    let local_user_view = get_local_user_view_from_jwt(&data.auth, context.pool()).await?;

    let community_id = data.community_id;
    let community = blocking(context.pool(), move |conn| {
      Community::read(conn, community_id)
    })
    .await??;
    let community_follower_form = CommunityFollowerForm {
      community_id: data.community_id,
      person_id: local_user_view.person.id,
      pending: false,
    };

    if community.local {
      if data.follow {
        check_community_ban(local_user_view.person.id, community_id, context.pool()).await?;

        let follow = move |conn: &'_ _| CommunityFollower::follow(conn, &community_follower_form);
        if blocking(context.pool(), follow).await?.is_err() {
          return Err(ApiError::err("community_follower_already_exists").into());
        }
      } else {
        let unfollow =
          move |conn: &'_ _| CommunityFollower::unfollow(conn, &community_follower_form);
        if blocking(context.pool(), unfollow).await?.is_err() {
          return Err(ApiError::err("community_follower_already_exists").into());
        }
      }
    } else if data.follow {
      // Dont actually add to the community followers here, because you need
      // to wait for the accept
      local_user_view
        .person
        .send_follow(&community.actor_id(), context)
        .await?;
    } else {
      local_user_view
        .person
        .send_unfollow(&community.actor_id(), context)
        .await?;
      let unfollow = move |conn: &'_ _| CommunityFollower::unfollow(conn, &community_follower_form);
      if blocking(context.pool(), unfollow).await?.is_err() {
        return Err(ApiError::err("community_follower_already_exists").into());
      }
    }

    let community_id = data.community_id;
    let person_id = local_user_view.person.id;
    let mut community_view = blocking(context.pool(), move |conn| {
      CommunityView::read(conn, community_id, Some(person_id))
    })
    .await??;

    // TODO: this needs to return a "pending" state, until Accept is received from the remote server
    // For now, just assume that remote follows are accepted.
    // Otherwise, the subscribed will be null
    if !community.local {
      community_view.subscribed = data.follow;
    }

    Ok(CommunityResponse { community_view })
  }
}

#[async_trait::async_trait(?Send)]
impl Perform for GetFollowedCommunities {
  type Response = GetFollowedCommunitiesResponse;

  async fn perform(
    &self,
    context: &Data<LemmyContext>,
    _websocket_id: Option<ConnectionId>,
  ) -> Result<GetFollowedCommunitiesResponse, LemmyError> {
    let data: &GetFollowedCommunities = &self;
    let local_user_view = get_local_user_view_from_jwt(&data.auth, context.pool()).await?;

    let person_id = local_user_view.person.id;
    let communities = match blocking(context.pool(), move |conn| {
      CommunityFollowerView::for_person(conn, person_id)
    })
    .await?
    {
      Ok(communities) => communities,
      _ => return Err(ApiError::err("system_err_login").into()),
    };

    // Return the jwt
    Ok(GetFollowedCommunitiesResponse { communities })
  }
}

#[async_trait::async_trait(?Send)]
impl Perform for BanFromCommunity {
  type Response = BanFromCommunityResponse;

  async fn perform(
    &self,
    context: &Data<LemmyContext>,
    websocket_id: Option<ConnectionId>,
  ) -> Result<BanFromCommunityResponse, LemmyError> {
    let data: &BanFromCommunity = &self;
    let local_user_view = get_local_user_view_from_jwt(&data.auth, context.pool()).await?;

    let community_id = data.community_id;
    let banned_person_id = data.person_id;

    // Verify that only mods or admins can ban
    is_mod_or_admin(context.pool(), local_user_view.person.id, community_id).await?;

    let community_user_ban_form = CommunityPersonBanForm {
      community_id: data.community_id,
      person_id: data.person_id,
    };

    if data.ban {
      let ban = move |conn: &'_ _| CommunityPersonBan::ban(conn, &community_user_ban_form);
      if blocking(context.pool(), ban).await?.is_err() {
        return Err(ApiError::err("community_user_already_banned").into());
      }

      // Also unsubscribe them from the community, if they are subscribed
      let community_follower_form = CommunityFollowerForm {
        community_id: data.community_id,
        person_id: banned_person_id,
        pending: false,
      };
      blocking(context.pool(), move |conn: &'_ _| {
        CommunityFollower::unfollow(conn, &community_follower_form)
      })
      .await?
      .ok();
    } else {
      let unban = move |conn: &'_ _| CommunityPersonBan::unban(conn, &community_user_ban_form);
      if blocking(context.pool(), unban).await?.is_err() {
        return Err(ApiError::err("community_user_already_banned").into());
      }
    }

    // Remove/Restore their data if that's desired
    if data.remove_data {
      // Posts
      blocking(context.pool(), move |conn: &'_ _| {
        Post::update_removed_for_creator(conn, banned_person_id, Some(community_id), true)
      })
      .await??;

      // Comments
      // TODO Diesel doesn't allow updates with joins, so this has to be a loop
      let comments = blocking(context.pool(), move |conn| {
        CommentQueryBuilder::create(conn)
          .creator_id(banned_person_id)
          .community_id(community_id)
          .limit(std::i64::MAX)
          .list()
      })
      .await??;

      for comment_view in &comments {
        let comment_id = comment_view.comment.id;
        blocking(context.pool(), move |conn: &'_ _| {
          Comment::update_removed(conn, comment_id, true)
        })
        .await??;
      }
    }

    // Mod tables
    // TODO eventually do correct expires
    let expires = match data.expires {
      Some(time) => Some(naive_from_unix(time)),
      None => None,
    };

    let form = ModBanFromCommunityForm {
      mod_person_id: local_user_view.person.id,
      other_person_id: data.person_id,
      community_id: data.community_id,
      reason: data.reason.to_owned(),
      banned: Some(data.ban),
      expires,
    };
    blocking(context.pool(), move |conn| {
      ModBanFromCommunity::create(conn, &form)
    })
    .await??;

    let person_id = data.person_id;
    let person_view = blocking(context.pool(), move |conn| {
      PersonViewSafe::read(conn, person_id)
    })
    .await??;

    let res = BanFromCommunityResponse {
      person_view,
      banned: data.ban,
    };

    context.chat_server().do_send(SendCommunityRoomMessage {
      op: UserOperation::BanFromCommunity,
      response: res.clone(),
      community_id,
      websocket_id,
    });

    Ok(res)
  }
}

#[async_trait::async_trait(?Send)]
impl Perform for AddModToCommunity {
  type Response = AddModToCommunityResponse;

  async fn perform(
    &self,
    context: &Data<LemmyContext>,
    websocket_id: Option<ConnectionId>,
  ) -> Result<AddModToCommunityResponse, LemmyError> {
    let data: &AddModToCommunity = &self;
    let local_user_view = get_local_user_view_from_jwt(&data.auth, context.pool()).await?;

    let community_moderator_form = CommunityModeratorForm {
      community_id: data.community_id,
      person_id: data.person_id,
    };

    let community_id = data.community_id;

    // Verify that only mods or admins can add mod
    is_mod_or_admin(context.pool(), local_user_view.person.id, community_id).await?;

    if data.added {
      let join = move |conn: &'_ _| CommunityModerator::join(conn, &community_moderator_form);
      if blocking(context.pool(), join).await?.is_err() {
        return Err(ApiError::err("community_moderator_already_exists").into());
      }
    } else {
      let leave = move |conn: &'_ _| CommunityModerator::leave(conn, &community_moderator_form);
      if blocking(context.pool(), leave).await?.is_err() {
        return Err(ApiError::err("community_moderator_already_exists").into());
      }
    }

    // Mod tables
    let form = ModAddCommunityForm {
      mod_person_id: local_user_view.person.id,
      other_person_id: data.person_id,
      community_id: data.community_id,
      removed: Some(!data.added),
    };
    blocking(context.pool(), move |conn| {
      ModAddCommunity::create(conn, &form)
    })
    .await??;

    let community_id = data.community_id;
    let moderators = blocking(context.pool(), move |conn| {
      CommunityModeratorView::for_community(conn, community_id)
    })
    .await??;

    let res = AddModToCommunityResponse { moderators };

    context.chat_server().do_send(SendCommunityRoomMessage {
      op: UserOperation::AddModToCommunity,
      response: res.clone(),
      community_id,
      websocket_id,
    });

    Ok(res)
  }
}

#[async_trait::async_trait(?Send)]
impl Perform for TransferCommunity {
  type Response = GetCommunityResponse;

  async fn perform(
    &self,
    context: &Data<LemmyContext>,
    _websocket_id: Option<ConnectionId>,
  ) -> Result<GetCommunityResponse, LemmyError> {
    let data: &TransferCommunity = &self;
    let local_user_view = get_local_user_view_from_jwt(&data.auth, context.pool()).await?;

    let community_id = data.community_id;
    let read_community = blocking(context.pool(), move |conn| {
      Community::read(conn, community_id)
    })
    .await??;

    let site_creator_id = blocking(context.pool(), move |conn| {
      Site::read(conn, 1).map(|s| s.creator_id)
    })
    .await??;

    let mut admins = blocking(context.pool(), move |conn| PersonViewSafe::admins(conn)).await??;

    // Making sure the creator, if an admin, is at the top
    let creator_index = admins
      .iter()
      .position(|r| r.person.id == site_creator_id)
      .context(location_info!())?;
    let creator_person = admins.remove(creator_index);
    admins.insert(0, creator_person);

    // Make sure user is the creator, or an admin
    if local_user_view.person.id != read_community.creator_id
      && !admins
        .iter()
        .map(|a| a.person.id)
        .any(|x| x == local_user_view.person.id)
    {
      return Err(ApiError::err("not_an_admin").into());
    }

    let community_id = data.community_id;
    let new_creator = data.person_id;
    let update = move |conn: &'_ _| Community::update_creator(conn, community_id, new_creator);
    if blocking(context.pool(), update).await?.is_err() {
      return Err(ApiError::err("couldnt_update_community").into());
    };

    // You also have to re-do the community_moderator table, reordering it.
    let community_id = data.community_id;
    let mut community_mods = blocking(context.pool(), move |conn| {
      CommunityModeratorView::for_community(conn, community_id)
    })
    .await??;
    let creator_index = community_mods
      .iter()
      .position(|r| r.moderator.id == data.person_id)
      .context(location_info!())?;
    let creator_person = community_mods.remove(creator_index);
    community_mods.insert(0, creator_person);

    let community_id = data.community_id;
    blocking(context.pool(), move |conn| {
      CommunityModerator::delete_for_community(conn, community_id)
    })
    .await??;

    // TODO: this should probably be a bulk operation
    for cmod in &community_mods {
      let community_moderator_form = CommunityModeratorForm {
        community_id: cmod.community.id,
        person_id: cmod.moderator.id,
      };

      let join = move |conn: &'_ _| CommunityModerator::join(conn, &community_moderator_form);
      if blocking(context.pool(), join).await?.is_err() {
        return Err(ApiError::err("community_moderator_already_exists").into());
      }
    }

    // Mod tables
    let form = ModAddCommunityForm {
      mod_person_id: local_user_view.person.id,
      other_person_id: data.person_id,
      community_id: data.community_id,
      removed: Some(false),
    };
    blocking(context.pool(), move |conn| {
      ModAddCommunity::create(conn, &form)
    })
    .await??;

    let community_id = data.community_id;
    let person_id = local_user_view.person.id;
    let community_view = match blocking(context.pool(), move |conn| {
      CommunityView::read(conn, community_id, Some(person_id))
    })
    .await?
    {
      Ok(community) => community,
      Err(_e) => return Err(ApiError::err("couldnt_find_community").into()),
    };

    let community_id = data.community_id;
    let moderators = match blocking(context.pool(), move |conn| {
      CommunityModeratorView::for_community(conn, community_id)
    })
    .await?
    {
      Ok(moderators) => moderators,
      Err(_e) => return Err(ApiError::err("couldnt_find_community").into()),
    };

    // Return the jwt
    Ok(GetCommunityResponse {
      community_view,
      moderators,
      online: 0,
    })
  }
}

fn send_community_websocket(
  res: &CommunityResponse,
  context: &Data<LemmyContext>,
  websocket_id: Option<ConnectionId>,
  op: UserOperation,
) {
  // Strip out the person id and subscribed when sending to others
  let mut res_sent = res.clone();
  res_sent.community_view.subscribed = false;

  context.chat_server().do_send(SendCommunityRoomMessage {
    op,
    response: res_sent,
    community_id: res.community_view.community.id,
    websocket_id,
  });
}
