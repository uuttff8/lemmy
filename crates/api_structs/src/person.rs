use lemmy_db_views::{
  comment_view::CommentView,
  post_view::PostView,
  private_message_view::PrivateMessageView,
};
use lemmy_db_views_actor::{
  community_follower_view::CommunityFollowerView,
  community_moderator_view::CommunityModeratorView,
  person_mention_view::PersonMentionView,
  person_view::PersonViewSafe,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Debug)]
pub struct Login {
  pub username_or_email: String,
  pub password: String,
}

#[derive(Deserialize)]
pub struct Register {
  pub username: String,
  pub email: Option<String>,
  pub password: String,
  pub password_verify: String,
  pub show_nsfw: bool,
  pub captcha_uuid: Option<String>,
  pub captcha_answer: Option<String>,
}

#[derive(Deserialize)]
pub struct GetCaptcha {}

#[derive(Serialize)]
pub struct GetCaptchaResponse {
  pub ok: Option<CaptchaResponse>, // Will be None if captchas are disabled
}

#[derive(Serialize)]
pub struct CaptchaResponse {
  pub png: String,         // A Base64 encoded png
  pub wav: Option<String>, // A Base64 encoded wav audio
  pub uuid: String,
}

#[derive(Deserialize)]
pub struct SaveUserSettings {
  pub show_nsfw: Option<bool>,
  pub theme: Option<String>,
  pub default_sort_type: Option<i16>,
  pub default_listing_type: Option<i16>,
  pub lang: Option<String>,
  pub avatar: Option<String>,
  pub banner: Option<String>,
  pub preferred_username: Option<String>,
  pub email: Option<String>,
  pub bio: Option<String>,
  pub matrix_user_id: Option<String>,
  pub new_password: Option<String>,
  pub new_password_verify: Option<String>,
  pub old_password: Option<String>,
  pub show_avatars: Option<bool>,
  pub send_notifications_to_email: Option<bool>,
  pub auth: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
  pub jwt: String,
}

#[derive(Deserialize)]
pub struct GetPersonDetails {
  pub person_id: Option<i32>,
  pub username: Option<String>,
  pub sort: String,
  pub page: Option<i64>,
  pub limit: Option<i64>,
  pub community_id: Option<i32>,
  pub saved_only: bool,
  pub auth: Option<String>,
}

#[derive(Serialize)]
pub struct GetPersonDetailsResponse {
  pub person_view: PersonViewSafe,
  pub follows: Vec<CommunityFollowerView>,
  pub moderates: Vec<CommunityModeratorView>,
  pub comments: Vec<CommentView>,
  pub posts: Vec<PostView>,
}

#[derive(Serialize)]
pub struct GetRepliesResponse {
  pub replies: Vec<CommentView>,
}

#[derive(Serialize)]
pub struct GetPersonMentionsResponse {
  pub mentions: Vec<PersonMentionView>,
}

#[derive(Deserialize)]
pub struct MarkAllAsRead {
  pub auth: String,
}

#[derive(Deserialize)]
pub struct AddAdmin {
  pub local_user_id: i32,
  pub added: bool,
  pub auth: String,
}

#[derive(Serialize, Clone)]
pub struct AddAdminResponse {
  pub admins: Vec<PersonViewSafe>,
}

#[derive(Deserialize)]
pub struct BanPerson {
  pub person_id: i32,
  pub ban: bool,
  pub remove_data: bool,
  pub reason: Option<String>,
  pub expires: Option<i64>,
  pub auth: String,
}

#[derive(Serialize, Clone)]
pub struct BanPersonResponse {
  pub person_view: PersonViewSafe,
  pub banned: bool,
}

#[derive(Deserialize)]
pub struct GetReplies {
  pub sort: String,
  pub page: Option<i64>,
  pub limit: Option<i64>,
  pub unread_only: bool,
  pub auth: String,
}

#[derive(Deserialize)]
pub struct GetPersonMentions {
  pub sort: String,
  pub page: Option<i64>,
  pub limit: Option<i64>,
  pub unread_only: bool,
  pub auth: String,
}

#[derive(Deserialize)]
pub struct MarkPersonMentionAsRead {
  pub person_mention_id: i32,
  pub read: bool,
  pub auth: String,
}

#[derive(Serialize, Clone)]
pub struct PersonMentionResponse {
  pub person_mention_view: PersonMentionView,
}

#[derive(Deserialize)]
pub struct DeleteAccount {
  pub password: String,
  pub auth: String,
}

#[derive(Deserialize)]
pub struct PasswordReset {
  pub email: String,
}

#[derive(Serialize, Clone)]
pub struct PasswordResetResponse {}

#[derive(Deserialize)]
pub struct PasswordChange {
  pub token: String,
  pub password: String,
  pub password_verify: String,
}

#[derive(Deserialize)]
pub struct CreatePrivateMessage {
  pub content: String,
  pub recipient_id: i32,
  pub auth: String,
}

#[derive(Deserialize)]
pub struct EditPrivateMessage {
  pub private_message_id: i32,
  pub content: String,
  pub auth: String,
}

#[derive(Deserialize)]
pub struct DeletePrivateMessage {
  pub private_message_id: i32,
  pub deleted: bool,
  pub auth: String,
}

#[derive(Deserialize)]
pub struct MarkPrivateMessageAsRead {
  pub private_message_id: i32,
  pub read: bool,
  pub auth: String,
}

#[derive(Deserialize)]
pub struct GetPrivateMessages {
  pub unread_only: bool,
  pub page: Option<i64>,
  pub limit: Option<i64>,
  pub auth: String,
}

#[derive(Serialize, Clone)]
pub struct PrivateMessagesResponse {
  pub private_messages: Vec<PrivateMessageView>,
}

#[derive(Serialize, Clone)]
pub struct PrivateMessageResponse {
  pub private_message_view: PrivateMessageView,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GetReportCount {
  pub community: Option<i32>,
  pub auth: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GetReportCountResponse {
  pub community: Option<i32>,
  pub comment_reports: i64,
  pub post_reports: i64,
}
