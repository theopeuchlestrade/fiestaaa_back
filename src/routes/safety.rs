use crate::{auth::AuthenticatedUser, state::AppState};
use actix_web::{HttpResponse, Responder, delete, get, post, web};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Deserialize, ToSchema)]
pub struct BlockPayload {
    pub public_id: Uuid,
}
#[derive(Deserialize, ToSchema)]
pub struct ReportPayload {
    pub public_id: Option<Uuid>,
    pub event_id: Option<i64>,
    pub reason: String,
    pub comment: String,
}
#[derive(Deserialize, ToSchema)]
pub struct ModerationAction {
    pub action: String,
    pub public_id: Option<Uuid>,
    pub event_id: Option<i64>,
    pub term: Option<String>,
}
fn unavailable() -> HttpResponse {
    HttpResponse::ServiceUnavailable().json(json!({"error":"safety_unavailable"}))
}
pub async fn ensure_contact(db: &sqlx::PgPool, a: i64, b: i64) -> Result<(), HttpResponse> {
    match sqlx::query_scalar::<_, bool>("SELECT fiestaaa_contact_blocked($1,$2)")
        .bind(a)
        .bind(b)
        .fetch_one(db)
        .await
    {
        Ok(false) => Ok(()),
        Ok(true) => Err(HttpResponse::Forbidden().json(json!({"error":"contact_unavailable"}))),
        Err(_) => Err(unavailable()),
    }
}
#[utoipa::path(get,path="/me/blocks",tag="users",responses((status=200,description="Own blocked users")))]
#[get("/me/blocks")]
pub async fn list_blocks(user: AuthenticatedUser, state: web::Data<AppState>) -> impl Responder {
    match sqlx::query("SELECT u.public_id,u.handle FROM user_blocks b JOIN users u ON u.id=b.blocked_id WHERE b.blocker_id=$1 ORDER BY b.created_at DESC").bind(user.id).fetch_all(&state.db).await{
        Ok(rows)=>HttpResponse::Ok().json(rows.iter().map(|r|json!({"public_id":r.get::<Uuid,_>("public_id"),"handle":r.get::<String,_>("handle")})).collect::<Vec<_>>()),Err(_)=>unavailable()
    }
}
#[utoipa::path(post,path="/me/blocks",tag="users",request_body=BlockPayload,responses((status=200,description="Contact blocked"),(status=400,description="Cannot block self")))]
#[post("/me/blocks")]
pub async fn block(
    user: AuthenticatedUser,
    state: web::Data<AppState>,
    payload: web::Json<BlockPayload>,
) -> impl Responder {
    if user.public_id == payload.public_id {
        return HttpResponse::BadRequest().finish();
    }
    let target = sqlx::query_scalar::<_, i64>("SELECT id FROM users WHERE public_id=$1")
        .bind(payload.public_id)
        .fetch_optional(&state.db)
        .await;
    let id = match target {
        Ok(Some(id)) => id,
        Ok(None) => return HttpResponse::NotFound().finish(),
        Err(_) => return unavailable(),
    };
    match sqlx::query(
        "INSERT INTO user_blocks(blocker_id,blocked_id) VALUES($1,$2) ON CONFLICT DO NOTHING",
    )
    .bind(user.id)
    .bind(id)
    .execute(&state.db)
    .await
    {
        Ok(_) => HttpResponse::Ok().json(json!({"status":"blocked"})),
        Err(_) => unavailable(),
    }
}
#[utoipa::path(delete,path="/me/blocks/{public_id}",tag="users",params(("public_id"=Uuid,Path,description="Blocked user")),responses((status=200,description="Unblocked, friendship is not recreated")))]
#[delete("/me/blocks/{public_id}")]
pub async fn unblock(
    user: AuthenticatedUser,
    state: web::Data<AppState>,
    public_id: web::Path<Uuid>,
) -> impl Responder {
    match sqlx::query("DELETE FROM user_blocks WHERE blocker_id=$1 AND blocked_id=(SELECT id FROM users WHERE public_id=$2)").bind(user.id).bind(*public_id).execute(&state.db).await{
        Ok(_)=>HttpResponse::Ok().json(json!({"status":"unblocked"})),Err(_)=>unavailable()
    }
}
#[utoipa::path(post,path="/reports",tag="users",request_body=ReportPayload,responses((status=201,description="Private report recorded"),(status=403,description="Content inaccessible"),(status=429,description="Rate limited")))]
#[post("/reports")]
pub async fn report(
    user: AuthenticatedUser,
    state: web::Data<AppState>,
    payload: web::Json<ReportPayload>,
) -> impl Responder {
    if !["harassment", "inappropriate", "spam", "other"].contains(&payload.reason.as_str())
        || payload.comment.len() > 4000
        || (payload.public_id.is_some() == payload.event_id.is_some())
    {
        return HttpResponse::BadRequest().finish();
    }
    if !state
        .auth_rate_limiter
        .allow(&format!("report:{}", user.id))
        .await
    {
        return HttpResponse::TooManyRequests().finish();
    }
    if let Some(event) = payload.event_id
        && let Err(r) =
            super::event_access::ensure_event_member_email(&state.db, event, &user.email).await
    {
        return r;
    }
    let target = if let Some(public) = payload.public_id {
        match sqlx::query_scalar::<_, i64>("SELECT id FROM users WHERE public_id=$1")
            .bind(public)
            .fetch_optional(&state.db)
            .await
        {
            Ok(Some(id)) => Some(id),
            Ok(None) => return HttpResponse::NotFound().finish(),
            Err(_) => return unavailable(),
        }
    } else {
        None
    };
    match sqlx::query_scalar::<_,Uuid>("INSERT INTO abuse_reports(reporter_id,target_user_id,event_id,reason,comment_ciphertext) VALUES($1,$2,$3,$4,fiestaaa_encrypt_text($5)) RETURNING public_id")
        .bind(user.id).bind(target).bind(payload.event_id).bind(&payload.reason).bind(&payload.comment).fetch_one(&state.db).await{
        Ok(id)=>{crate::observability::capture_message(sentry::Level::Info,"abuse_report_received");HttpResponse::Created().json(json!({"public_id":id,"status":"received"}))},Err(_)=>unavailable()
    }
}
#[utoipa::path(get,path="/admin/reports",tag="moderation",responses((status=200,description="Private reports, administrators only"),(status=403,description="Administrator required")))]
#[get("/admin/reports")]
pub async fn reports(user: AuthenticatedUser, state: web::Data<AppState>) -> impl Responder {
    if !state.admin_emails.contains(&user.email.to_lowercase()) {
        return HttpResponse::Forbidden().finish();
    }
    match sqlx::query("SELECT r.public_id,r.event_id,r.reason,fiestaaa_decrypt_text(r.comment_ciphertext) AS comment,u.public_id AS target FROM abuse_reports r LEFT JOIN users u ON u.id=r.target_user_id WHERE r.status='open' ORDER BY r.created_at LIMIT 100").fetch_all(&state.db).await{
        Ok(rows)=>HttpResponse::Ok().insert_header(("Cache-Control","no-store")).json(rows.iter().map(|r|json!({"public_id":r.get::<Uuid,_>("public_id"),"event_id":r.get::<Option<i64>,_>("event_id"),"reason":r.get::<String,_>("reason"),"comment":r.get::<String,_>("comment"),"target":r.get::<Option<Uuid>,_>("target")})).collect::<Vec<_>>()),Err(_)=>unavailable()
    }
}
#[utoipa::path(post,path="/admin/moderation",tag="moderation",request_body=ModerationAction,responses((status=200,description="Action applied"),(status=403,description="Administrator required")))]
#[post("/admin/moderation")]
pub async fn moderate(
    user: AuthenticatedUser,
    state: web::Data<AppState>,
    payload: web::Json<ModerationAction>,
) -> impl Responder {
    if !state.admin_emails.contains(&user.email.to_lowercase()) {
        return HttpResponse::Forbidden().finish();
    }
    if payload.action == "hide_avatar" {
        let Some(public_id) = payload.public_id else {
            return HttpResponse::BadRequest().finish();
        };
        let mut tx = match state.db.begin().await {
            Ok(t) => t,
            Err(_) => return unavailable(),
        };
        let avatar = match sqlx::query_scalar::<_, Option<String>>(
            "SELECT avatar_url FROM users WHERE public_id=$1 FOR UPDATE",
        )
        .bind(public_id)
        .fetch_optional(&mut *tx)
        .await
        {
            Ok(Some(a)) => a,
            Ok(None) => return HttpResponse::NotFound().finish(),
            Err(_) => return unavailable(),
        };
        if sqlx::query("UPDATE users SET avatar_url=NULL WHERE public_id=$1")
            .bind(public_id)
            .execute(&mut *tx)
            .await
            .is_err()
            || tx.commit().await.is_err()
        {
            return unavailable();
        }
        super::users::cleanup_avatar_file(&state, avatar.as_deref()).await;
        return HttpResponse::Ok().json(json!({"status":"applied"}));
    }
    let result=match payload.action.as_str(){
        "suspend"|"unsuspend" if payload.public_id.is_some()=>sqlx::query("UPDATE users SET suspended=$2,session_version=session_version+1 WHERE public_id=$1 AND id<>$3").bind(payload.public_id).bind(payload.action=="suspend").bind(user.id).execute(&state.db).await,
        "hide_event" if payload.event_id.is_some()=>sqlx::query("UPDATE events SET moderation_hidden=TRUE,deleted_at=NOW(),deletion_reason='moderation',purge_at=NULL WHERE event_id=$1").bind(payload.event_id).execute(&state.db).await,
        "resolve"|"dismiss" if payload.public_id.is_some()=>sqlx::query("UPDATE abuse_reports SET status=$2,resolved_at=NOW() WHERE public_id=$1").bind(payload.public_id).bind(if payload.action=="resolve"{"resolved"}else{"dismissed"}).execute(&state.db).await,
        "add_term" if payload.term.as_ref().is_some_and(|t|t.trim().chars().count()>=3 && t.len()<=200)=>sqlx::query("INSERT INTO moderation_terms(term) VALUES(lower(trim($1))) ON CONFLICT DO NOTHING").bind(&payload.term).execute(&state.db).await,
        "remove_term" if payload.term.is_some()=>sqlx::query("DELETE FROM moderation_terms WHERE term=lower(trim($1))").bind(&payload.term).execute(&state.db).await,
        _=>return HttpResponse::BadRequest().finish(),
    };
    match result {
        Ok(_) => {
            log::info!("moderation action {} by {}", payload.action, user.public_id);
            HttpResponse::Ok().json(json!({"status":"applied"}))
        }
        Err(_) => unavailable(),
    }
}
#[derive(Deserialize)]
pub struct HandleQuery {
    pub handle: String,
}
#[utoipa::path(get,path="/safety/user",tag="users",params(("handle"=String,Query,description="Exact public handle")),responses((status=200,description="Public identity"),(status=404,description="Unknown handle")))]
#[get("/safety/user")]
pub async fn resolve_user(
    user: AuthenticatedUser,
    state: web::Data<AppState>,
    query: web::Query<HandleQuery>,
) -> impl Responder {
    if !state
        .auth_rate_limiter
        .allow(&format!("safety-lookup:{}", user.id))
        .await
    {
        return HttpResponse::TooManyRequests().finish();
    }
    match sqlx::query("SELECT public_id,handle FROM users WHERE lower(handle)=lower($1) AND id<>$2")
        .bind(query.handle.trim())
        .bind(user.id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(r)) => HttpResponse::Ok().json(
            json!({"public_id":r.get::<Uuid,_>("public_id"),"handle":r.get::<String,_>("handle")}),
        ),
        Ok(None) => HttpResponse::NotFound().finish(),
        Err(_) => unavailable(),
    }
}
