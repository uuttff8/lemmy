#[macro_use]
extern crate lazy_static;

pub mod activities;
pub mod activity_queue;
pub mod extensions;
pub mod fetcher;
pub mod http;
pub mod inbox;
pub mod objects;
pub mod routes;

use crate::extensions::{
  group_extensions::GroupExtension,
  page_extension::PageExtension,
  signatures::{PublicKey, PublicKeyExtension},
};
use activitystreams::{
  activity::Follow,
  actor::{ApActor, Group, Person},
  base::AnyBase,
  object::{ApObject, Note, Page},
};
use activitystreams_ext::{Ext1, Ext2};
use anyhow::{anyhow, Context};
use diesel::NotFound;
use lemmy_api_structs::blocking;
use lemmy_db_queries::{source::activity::Activity_, ApubObject, DbPool};
use lemmy_db_schema::{
  source::{
    activity::Activity,
    comment::Comment,
    community::Community,
    person::Person as DbPerson,
    post::Post,
    private_message::PrivateMessage,
  },
  DbUrl,
};
use lemmy_utils::{location_info, settings::structs::Settings, LemmyError};
use lemmy_websocket::LemmyContext;
use serde::Serialize;
use std::net::IpAddr;
use url::{ParseError, Url};

/// Activitystreams type for community
type GroupExt = Ext2<ApActor<ApObject<Group>>, GroupExtension, PublicKeyExtension>;
/// Activitystreams type for person
type PersonExt = Ext1<ApActor<ApObject<Person>>, PublicKeyExtension>;
/// Activitystreams type for post
type PageExt = Ext1<ApObject<Page>, PageExtension>;
type NoteExt = ApObject<Note>;

pub static APUB_JSON_CONTENT_TYPE: &str = "application/activity+json";

/// Checks if the ID is allowed for sending or receiving.
///
/// In particular, it checks for:
/// - federation being enabled (if its disabled, only local URLs are allowed)
/// - the correct scheme (either http or https)
/// - URL being in the allowlist (if it is active)
/// - URL not being in the blocklist (if it is active)
///
/// Note that only one of allowlist and blacklist can be enabled, not both.
fn check_is_apub_id_valid(apub_id: &Url) -> Result<(), LemmyError> {
  let settings = Settings::get();
  let domain = apub_id.domain().context(location_info!())?.to_string();
  let local_instance = settings.get_hostname_without_port()?;

  if !settings.federation().enabled {
    return if domain == local_instance {
      Ok(())
    } else {
      Err(
        anyhow!(
          "Trying to connect with {}, but federation is disabled",
          domain
        )
        .into(),
      )
    };
  }

  let host = apub_id.host_str().context(location_info!())?;
  let host_as_ip = host.parse::<IpAddr>();
  if host == "localhost" || host_as_ip.is_ok() {
    return Err(anyhow!("invalid hostname {}: {}", host, apub_id).into());
  }

  if apub_id.scheme() != Settings::get().get_protocol_string() {
    return Err(anyhow!("invalid apub id scheme {}: {}", apub_id.scheme(), apub_id).into());
  }

  let allowed_instances = Settings::get().get_allowed_instances();
  let blocked_instances = Settings::get().get_blocked_instances();

  if allowed_instances.is_none() && blocked_instances.is_none() {
    Ok(())
  } else if let Some(mut allowed) = allowed_instances {
    // need to allow this explicitly because apub receive might contain objects from our local
    // instance. split is needed to remove the port in our federation test setup.
    allowed.push(local_instance);

    if allowed.contains(&domain) {
      Ok(())
    } else {
      Err(anyhow!("{} not in federation allowlist", domain).into())
    }
  } else if let Some(blocked) = blocked_instances {
    if blocked.contains(&domain) {
      Err(anyhow!("{} is in federation blocklist", domain).into())
    } else {
      Ok(())
    }
  } else {
    panic!("Invalid config, both allowed_instances and blocked_instances are specified");
  }
}

/// Common functions for ActivityPub objects, which are implemented by most (but not all) objects
/// and actors in Lemmy.
#[async_trait::async_trait(?Send)]
pub trait ApubObjectType {
  async fn send_create(&self, creator: &DbPerson, context: &LemmyContext)
    -> Result<(), LemmyError>;
  async fn send_update(&self, creator: &DbPerson, context: &LemmyContext)
    -> Result<(), LemmyError>;
  async fn send_delete(&self, creator: &DbPerson, context: &LemmyContext)
    -> Result<(), LemmyError>;
  async fn send_undo_delete(
    &self,
    creator: &DbPerson,
    context: &LemmyContext,
  ) -> Result<(), LemmyError>;
  async fn send_remove(&self, mod_: &DbPerson, context: &LemmyContext) -> Result<(), LemmyError>;
  async fn send_undo_remove(
    &self,
    mod_: &DbPerson,
    context: &LemmyContext,
  ) -> Result<(), LemmyError>;
}

#[async_trait::async_trait(?Send)]
pub trait ApubLikeableType {
  async fn send_like(&self, creator: &DbPerson, context: &LemmyContext) -> Result<(), LemmyError>;
  async fn send_dislike(
    &self,
    creator: &DbPerson,
    context: &LemmyContext,
  ) -> Result<(), LemmyError>;
  async fn send_undo_like(
    &self,
    creator: &DbPerson,
    context: &LemmyContext,
  ) -> Result<(), LemmyError>;
}

/// Common methods provided by ActivityPub actors (community and person). Not all methods are
/// implemented by all actors.
#[async_trait::async_trait(?Send)]
pub trait ActorType {
  fn is_local(&self) -> bool;
  fn actor_id(&self) -> Url;

  // TODO: every actor should have a public key, so this shouldnt be an option (needs to be fixed in db)
  fn public_key(&self) -> Option<String>;
  fn private_key(&self) -> Option<String>;

  async fn send_follow(
    &self,
    follow_actor_id: &Url,
    context: &LemmyContext,
  ) -> Result<(), LemmyError>;
  async fn send_unfollow(
    &self,
    follow_actor_id: &Url,
    context: &LemmyContext,
  ) -> Result<(), LemmyError>;

  async fn send_accept_follow(
    &self,
    follow: Follow,
    context: &LemmyContext,
  ) -> Result<(), LemmyError>;

  async fn send_delete(&self, context: &LemmyContext) -> Result<(), LemmyError>;
  async fn send_undo_delete(&self, context: &LemmyContext) -> Result<(), LemmyError>;

  async fn send_remove(&self, context: &LemmyContext) -> Result<(), LemmyError>;
  async fn send_undo_remove(&self, context: &LemmyContext) -> Result<(), LemmyError>;

  async fn send_announce(
    &self,
    activity: AnyBase,
    context: &LemmyContext,
  ) -> Result<(), LemmyError>;

  /// For a given community, returns the inboxes of all followers.
  async fn get_follower_inboxes(&self, pool: &DbPool) -> Result<Vec<Url>, LemmyError>;

  fn get_shared_inbox_or_inbox_url(&self) -> Url;

  /// Outbox URL is not generally used by Lemmy, so it can be generated on the fly (but only for
  /// local actors).
  fn get_outbox_url(&self) -> Result<Url, LemmyError> {
    if !self.is_local() {
      return Err(anyhow!("get_outbox_url() called for remote actor").into());
    }
    Ok(Url::parse(&format!("{}/outbox", &self.actor_id()))?)
  }

  fn get_public_key_ext(&self) -> Result<PublicKeyExtension, LemmyError> {
    Ok(
      PublicKey {
        id: format!("{}#main-key", self.actor_id()),
        owner: self.actor_id(),
        public_key_pem: self.public_key().context(location_info!())?,
      }
      .to_ext(),
    )
  }
}

pub enum EndpointType {
  Community,
  Person,
  Post,
  Comment,
  PrivateMessage,
}

/// Generates the ActivityPub ID for a given object type and ID.
pub fn generate_apub_endpoint(
  endpoint_type: EndpointType,
  name: &str,
) -> Result<DbUrl, ParseError> {
  let point = match endpoint_type {
    EndpointType::Community => "c",
    EndpointType::Person => "u",
    EndpointType::Post => "post",
    EndpointType::Comment => "comment",
    EndpointType::PrivateMessage => "private_message",
  };

  Ok(
    Url::parse(&format!(
      "{}/{}/{}",
      Settings::get().get_protocol_and_hostname(),
      point,
      name
    ))?
    .into(),
  )
}

pub fn generate_followers_url(actor_id: &DbUrl) -> Result<DbUrl, ParseError> {
  Ok(Url::parse(&format!("{}/followers", actor_id))?.into())
}

pub fn generate_inbox_url(actor_id: &DbUrl) -> Result<DbUrl, ParseError> {
  Ok(Url::parse(&format!("{}/inbox", actor_id))?.into())
}

pub fn generate_shared_inbox_url(actor_id: &DbUrl) -> Result<DbUrl, LemmyError> {
  let actor_id = actor_id.clone().into_inner();
  let url = format!(
    "{}://{}{}/inbox",
    &actor_id.scheme(),
    &actor_id.host_str().context(location_info!())?,
    if let Some(port) = actor_id.port() {
      format!(":{}", port)
    } else {
      "".to_string()
    },
  );
  Ok(Url::parse(&url)?.into())
}

/// Store a sent or received activity in the database, for logging purposes. These records are not
/// persistent.
pub(crate) async fn insert_activity<T>(
  ap_id: &Url,
  activity: T,
  local: bool,
  sensitive: bool,
  pool: &DbPool,
) -> Result<(), LemmyError>
where
  T: Serialize + std::fmt::Debug + Send + 'static,
{
  let ap_id = ap_id.to_owned().into();
  blocking(pool, move |conn| {
    Activity::insert(conn, ap_id, &activity, local, sensitive)
  })
  .await??;
  Ok(())
}

pub(crate) enum PostOrComment {
  Comment(Box<Comment>),
  Post(Box<Post>),
}

/// Tries to find a post or comment in the local database, without any network requests.
/// This is used to handle deletions and removals, because in case we dont have the object, we can
/// simply ignore the activity.
pub(crate) async fn find_post_or_comment_by_id(
  context: &LemmyContext,
  apub_id: Url,
) -> Result<PostOrComment, LemmyError> {
  let ap_id = apub_id.clone();
  let post = blocking(context.pool(), move |conn| {
    Post::read_from_apub_id(conn, &ap_id.into())
  })
  .await?;
  if let Ok(p) = post {
    return Ok(PostOrComment::Post(Box::new(p)));
  }

  let ap_id = apub_id.clone();
  let comment = blocking(context.pool(), move |conn| {
    Comment::read_from_apub_id(conn, &ap_id.into())
  })
  .await?;
  if let Ok(c) = comment {
    return Ok(PostOrComment::Comment(Box::new(c)));
  }

  Err(NotFound.into())
}

pub(crate) enum Object {
  Comment(Box<Comment>),
  Post(Box<Post>),
  Community(Box<Community>),
  Person(Box<DbPerson>),
  PrivateMessage(Box<PrivateMessage>),
}

pub(crate) async fn find_object_by_id(
  context: &LemmyContext,
  apub_id: Url,
) -> Result<Object, LemmyError> {
  let ap_id = apub_id.clone();
  if let Ok(pc) = find_post_or_comment_by_id(context, ap_id.to_owned()).await {
    return Ok(match pc {
      PostOrComment::Post(p) => Object::Post(Box::new(*p)),
      PostOrComment::Comment(c) => Object::Comment(Box::new(*c)),
    });
  }

  let ap_id = apub_id.clone();
  let person = blocking(context.pool(), move |conn| {
    DbPerson::read_from_apub_id(conn, &ap_id.into())
  })
  .await?;
  if let Ok(u) = person {
    return Ok(Object::Person(Box::new(u)));
  }

  let ap_id = apub_id.clone();
  let community = blocking(context.pool(), move |conn| {
    Community::read_from_apub_id(conn, &ap_id.into())
  })
  .await?;
  if let Ok(c) = community {
    return Ok(Object::Community(Box::new(c)));
  }

  let private_message = blocking(context.pool(), move |conn| {
    PrivateMessage::read_from_apub_id(conn, &apub_id.into())
  })
  .await?;
  if let Ok(pm) = private_message {
    return Ok(Object::PrivateMessage(Box::new(pm)));
  }

  Err(NotFound.into())
}
