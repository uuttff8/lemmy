use crate::{
  schema::{person, person_alias_1, person_alias_2},
  DbUrl,
};
use serde::Serialize;

#[derive(Clone, Queryable, Identifiable, PartialEq, Debug, Serialize)]
#[table_name = "person"]
pub struct Person {
  pub id: i32,
  pub name: String,
  pub preferred_username: Option<String>,
  pub avatar: Option<DbUrl>,
  pub banned: bool,
  pub published: chrono::NaiveDateTime,
  pub updated: Option<chrono::NaiveDateTime>,
  pub actor_id: DbUrl,
  pub bio: Option<String>,
  pub local: bool,
  pub private_key: Option<String>,
  pub public_key: Option<String>,
  pub last_refreshed_at: chrono::NaiveDateTime,
  pub banner: Option<DbUrl>,
  pub deleted: bool,
  pub inbox_url: DbUrl,
  pub shared_inbox_url: Option<DbUrl>,
}

/// A safe representation of person, without the sensitive info
#[derive(Clone, Queryable, Identifiable, PartialEq, Debug, Serialize)]
#[table_name = "person"]
pub struct PersonSafe {
  pub id: i32,
  pub name: String,
  pub preferred_username: Option<String>,
  pub avatar: Option<DbUrl>,
  pub banned: bool,
  pub published: chrono::NaiveDateTime,
  pub updated: Option<chrono::NaiveDateTime>,
  pub actor_id: DbUrl,
  pub bio: Option<String>,
  pub local: bool,
  pub banner: Option<DbUrl>,
  pub deleted: bool,
  pub inbox_url: DbUrl,
  pub shared_inbox_url: Option<DbUrl>,
}

#[derive(Clone, Queryable, Identifiable, PartialEq, Debug, Serialize)]
#[table_name = "person_alias_1"]
pub struct PersonAlias1 {
  pub id: i32,
  pub name: String,
  pub preferred_username: Option<String>,
  pub avatar: Option<DbUrl>,
  pub banned: bool,
  pub published: chrono::NaiveDateTime,
  pub updated: Option<chrono::NaiveDateTime>,
  pub actor_id: DbUrl,
  pub bio: Option<String>,
  pub local: bool,
  pub private_key: Option<String>,
  pub public_key: Option<String>,
  pub last_refreshed_at: chrono::NaiveDateTime,
  pub banner: Option<DbUrl>,
  pub deleted: bool,
  pub inbox_url: DbUrl,
  pub shared_inbox_url: Option<DbUrl>,
}

#[derive(Clone, Queryable, Identifiable, PartialEq, Debug, Serialize)]
#[table_name = "person_alias_1"]
pub struct PersonSafeAlias1 {
  pub id: i32,
  pub name: String,
  pub preferred_username: Option<String>,
  pub avatar: Option<DbUrl>,
  pub banned: bool,
  pub published: chrono::NaiveDateTime,
  pub updated: Option<chrono::NaiveDateTime>,
  pub actor_id: DbUrl,
  pub bio: Option<String>,
  pub local: bool,
  pub banner: Option<DbUrl>,
  pub deleted: bool,
  pub inbox_url: DbUrl,
  pub shared_inbox_url: Option<DbUrl>,
}

#[derive(Clone, Queryable, Identifiable, PartialEq, Debug, Serialize)]
#[table_name = "person_alias_2"]
pub struct PersonAlias2 {
  pub id: i32,
  pub name: String,
  pub preferred_username: Option<String>,
  pub avatar: Option<DbUrl>,
  pub banned: bool,
  pub published: chrono::NaiveDateTime,
  pub updated: Option<chrono::NaiveDateTime>,
  pub actor_id: DbUrl,
  pub bio: Option<String>,
  pub local: bool,
  pub private_key: Option<String>,
  pub public_key: Option<String>,
  pub last_refreshed_at: chrono::NaiveDateTime,
  pub banner: Option<DbUrl>,
  pub deleted: bool,
  pub inbox_url: DbUrl,
  pub shared_inbox_url: Option<DbUrl>,
}

#[derive(Clone, Queryable, Identifiable, PartialEq, Debug, Serialize)]
#[table_name = "person_alias_1"]
pub struct PersonSafeAlias2 {
  pub id: i32,
  pub name: String,
  pub preferred_username: Option<String>,
  pub avatar: Option<DbUrl>,
  pub banned: bool,
  pub published: chrono::NaiveDateTime,
  pub updated: Option<chrono::NaiveDateTime>,
  pub actor_id: DbUrl,
  pub bio: Option<String>,
  pub local: bool,
  pub banner: Option<DbUrl>,
  pub deleted: bool,
  pub inbox_url: DbUrl,
  pub shared_inbox_url: Option<DbUrl>,
}

#[derive(Insertable, AsChangeset, Clone)]
#[table_name = "person"]
pub struct PersonForm {
  pub name: String,
  pub preferred_username: Option<Option<String>>,
  pub avatar: Option<Option<DbUrl>>,
  pub banned: Option<bool>,
  pub published: Option<chrono::NaiveDateTime>,
  pub updated: Option<chrono::NaiveDateTime>,
  pub actor_id: Option<DbUrl>,
  pub bio: Option<Option<String>>,
  pub local: Option<bool>,
  pub private_key: Option<Option<String>>,
  pub public_key: Option<Option<String>>,
  pub last_refreshed_at: Option<chrono::NaiveDateTime>,
  pub banner: Option<Option<DbUrl>>,
  pub deleted: Option<bool>,
  pub inbox_url: Option<DbUrl>,
  pub shared_inbox_url: Option<Option<DbUrl>>,
}
