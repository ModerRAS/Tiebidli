//! 最小后台。能力对齐 Typecho 1.2 核心，不复刻 PHP 皮肤。
//! 注册只在 `allowRegister` 打开时可用。1.2 没有默认的找回密码。

use crate::auth::{self, SessionUser};
use crate::http::AppState;
use crate::links;
use crate::model::Site;
use crate::text::html_escape;
use crate::write::{ContentInput, MetaInput, UserInput};
use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use std::collections::HashMap;
use std::sync::Arc;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/admin", get(dashboard))
        .route("/admin/login", get(login_form).post(login_submit))
        .route("/admin/logout", post(logout))
        .route("/admin/register", get(register_form).post(register_submit))
        .route("/admin/posts", get(content_list))
        .route("/admin/posts/new", get(content_new).post(content_save_new))
        .route("/admin/posts/:cid", get(content_edit).post(content_save))
        .route("/admin/posts/:cid/delete", post(content_delete))
        .route("/admin/pages", get(page_list))
        .route("/admin/pages/new", get(page_new).post(page_save_new))
        .route("/admin/pages/:cid", get(page_edit).post(page_save))
        .route("/admin/pages/:cid/delete", post(page_delete))
        .route("/admin/categories", get(categories).post(category_save))
        .route("/admin/categories/:mid/delete", post(category_delete))
        .route("/admin/tags", get(tags).post(tag_save))
        .route("/admin/tags/:mid/delete", post(tag_delete))
        .route("/admin/comments", get(comments))
        .route("/admin/comments/:coid", post(comment_act))
        .route("/admin/media", get(media).post(media_upload))
        .route("/admin/media/:cid/delete", post(media_delete))
        .route("/admin/users", get(users).post(user_save))
        .route("/admin/users/:uid/delete", post(user_delete))
        .route("/admin/settings", get(settings).post(settings_save))
}

async fn dashboard(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let Ok((site, user, _)) = need(&state, &headers, "subscriber").await else {
        return redirect("/admin/login");
    };
    let body = format!(
        "<h1>控制台</h1><p>当前用户 {}（{}）</p><ul>{}</ul>",
        html_escape(&user.label()),
        html_escape(&user.group),
        nav_items(&user)
    );
    page(&site, &user, "控制台", &body)
}

async fn login_form(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if session(&state, &headers).await.ok().flatten().is_some() {
        return redirect("/admin");
    }
    let html = r#"<h1>登录</h1>
<form method="post" action="/admin/login" class="stack">
<label>用户名或邮箱</label><input name="name" required>
<label>密码</label><input name="password" type="password" required>
<button type="submit">登录</button>
</form>"#;
    bare("登录", html)
}

async fn login_submit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let form = pairs(&body);
    let name = form.get("name").map(|s| s.as_str()).unwrap_or("");
    let password = form.get("password").map(|s| s.as_str()).unwrap_or("");
    let Ok(Some(user)) = auth::login_user(&state.db, name, password).await else {
        return bare("登录", "<p class=\"warn\">用户名或密码错误</p><p><a href=\"/admin/login\">返回</a></p>");
    };
    let Ok(code) = auth::commit_login(&state.db, user.uid).await else {
        return plain(StatusCode::INTERNAL_SERVER_ERROR, "登录失败");
    };
    let root = root_of(&state, &headers).await;
    let mut extra = HeaderMap::new();
    push_cookie(&mut extra, &auth::cookie_name(&root, "__typecho_uid"), &user.uid.to_string(), Some(30 * 24 * 3600));
    push_cookie(&mut extra, &auth::cookie_name(&root, "__typecho_authCode"), &code, Some(30 * 24 * 3600));
    redirect_with("/admin", &extra)
}

async fn logout(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let root = root_of(&state, &headers).await;
    if let Ok(Some(user)) = session(&state, &headers).await {
        let sql = format!("UPDATE {} SET authCode = NULL WHERE uid = ?", state.db.table("users"));
        let _ = sqlx::query(&sql).bind(user.uid).execute(&state.db.pool).await;
    }
    let mut extra = HeaderMap::new();
    push_cookie(&mut extra, &auth::cookie_name(&root, "__typecho_uid"), "", Some(0));
    push_cookie(&mut extra, &auth::cookie_name(&root, "__typecho_authCode"), "", Some(0));
    redirect_with("/admin/login", &extra)
}

async fn register_form(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let Ok(site) = load_site(&state).await else {
        return plain(StatusCode::INTERNAL_SERVER_ERROR, "读取设置失败");
    };
    if !site.flag("allowRegister", false) {
        return plain(StatusCode::NOT_FOUND, "未开放注册");
    }
    let _ = headers;
    bare(
        "注册",
        r#"<h1>注册</h1><form method="post" action="/admin/register" class="stack">
<label>用户名</label><input name="name" required>
<label>邮箱</label><input name="mail" type="email" required>
<label>密码</label><input name="password" type="password" required>
<button type="submit">注册</button></form>"#,
    )
}

async fn register_submit(State(state): State<Arc<AppState>>, body: String) -> Response {
    let Ok(site) = load_site(&state).await else {
        return plain(StatusCode::INTERNAL_SERVER_ERROR, "读取设置失败");
    };
    if !site.flag("allowRegister", false) {
        return plain(StatusCode::NOT_FOUND, "未开放注册");
    }
    let form = pairs(&body);
    let name = form.get("name").cloned().unwrap_or_default();
    let mail = form.get("mail").cloned().unwrap_or_default();
    let password = form.get("password").cloned().unwrap_or_default();
    if name.trim().is_empty() || password.is_empty() || mail.trim().is_empty() {
        return bare("注册", "<p class=\"warn\">请填写用户名、邮箱和密码</p>");
    }
    let now = chrono::Utc::now().timestamp();
    match state
        .db
        .save_user(&UserInput {
            uid: None,
            name: name.trim().into(),
            password,
            mail: mail.trim().into(),
            url: String::new(),
            screen: name.trim().into(),
            group: "subscriber".into(),
            created: now,
        })
        .await
    {
        Ok(_) => bare("注册", "<p>注册成功，请<a href=\"/admin/login\">登录</a></p>"),
        Err(_) => bare("注册", "<p class=\"warn\">注册失败，用户名或邮箱可能已存在</p>"),
    }
}

async fn content_list(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    list_kind(&state, &headers, "post", "文章", "/admin/posts").await
}
async fn page_list(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    list_kind(&state, &headers, "page", "独立页", "/admin/pages").await
}

async fn list_kind(state: &AppState, headers: &HeaderMap, kind: &str, title: &str, base: &str) -> Response {
    let Ok((site, user, _)) = need(state, headers, "contributor").await else {
        return redirect("/admin/login");
    };
    if kind == "page" && !user.pass("editor") {
        return plain(StatusCode::FORBIDDEN, "独立页需要编辑权限");
    }
    let Ok(rows) = state.db.list_contents(kind, 100).await else {
        return plain(StatusCode::INTERNAL_SERVER_ERROR, "读取失败");
    };
    let mut table = String::from("<table><tr><th>标题</th><th>状态</th><th>时间</th><th></th></tr>");
    for row in rows {
        if !user.pass("editor") && row.author_id != user.uid {
            continue;
        }
        table.push_str(&format!(
            "<tr><td><a href=\"{base}/{}\">{}</a></td><td>{}</td><td>{}</td><td><form method=\"post\" action=\"{base}/{}/delete\"><button>删除</button></form></td></tr>",
            row.cid,
            html_escape(&row.title),
            html_escape(&row.status),
            row.created,
            row.cid
        ));
    }
    table.push_str("</table>");
    page(&site, &user, title, &format!("<p><a href=\"{base}/new\">新建</a></p>{table}"))
}

async fn content_new(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    editor(&state, &headers, "post", None).await
}
async fn page_new(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    editor(&state, &headers, "page", None).await
}
async fn content_edit(State(state): State<Arc<AppState>>, headers: HeaderMap, Path(cid): Path<i64>) -> Response {
    editor(&state, &headers, "post", Some(cid)).await
}
async fn page_edit(State(state): State<Arc<AppState>>, headers: HeaderMap, Path(cid): Path<i64>) -> Response {
    editor(&state, &headers, "page", Some(cid)).await
}

async fn editor(state: &AppState, headers: &HeaderMap, kind: &str, cid: Option<i64>) -> Response {
    let Ok((site, user, _)) = need(state, headers, "contributor").await else {
        return redirect("/admin/login");
    };
    if kind == "page" && !user.pass("editor") {
        return plain(StatusCode::FORBIDDEN, "独立页需要编辑权限");
    }
    let existing = if let Some(cid) = cid {
        match state.db.content_by_id(cid).await {
            Ok(Some(post)) if post.kind == kind && (user.pass("editor") || post.author_id == user.uid) => Some(post),
            Ok(Some(_)) => return plain(StatusCode::FORBIDDEN, "不能编辑这条内容"),
            _ => return plain(StatusCode::NOT_FOUND, "内容不存在"),
        }
    } else {
        None
    };
    let action = match (kind, cid) {
        ("page", Some(cid)) => format!("/admin/pages/{cid}"),
        ("page", None) => "/admin/pages/new".into(),
        (_, Some(cid)) => format!("/admin/posts/{cid}"),
        _ => "/admin/posts/new".into(),
    };
    let title = existing.as_ref().map(|p| p.title.as_str()).unwrap_or("");
    let slug = existing.as_ref().map(|p| p.slug.as_str()).unwrap_or("");
    let text = existing.as_ref().map(|p| p.text.as_str()).unwrap_or("");
    let status = existing.as_ref().map(|p| p.status.as_str()).unwrap_or("draft");
    let password = existing.as_ref().map(|p| p.password.as_str()).unwrap_or("");
    let created = existing.as_ref().map(|p| p.created).unwrap_or_else(|| chrono::Utc::now().timestamp());
    let local = created + site.timezone();
    let created_text = chrono::DateTime::from_timestamp(local, 0)
        .map(|d| d.format("%Y-%m-%dT%H:%M").to_string())
        .unwrap_or_default();
    let cats = state.db.list_metas("category").await.unwrap_or_default();
    let selected: Vec<i64> = existing
        .as_ref()
        .map(|p| p.categories.iter().map(|c| c.mid).collect())
        .unwrap_or_default();
    let cat_text = selected.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(",");
    let tag_text = existing
        .as_ref()
        .map(|p| p.tags.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(","))
        .unwrap_or_default();
    let mut options = String::new();
    for name in ["publish", "draft", "waiting", "hidden", "private"] {
        let sel = if name == status { " selected" } else { "" };
        options.push_str(&format!("<option value=\"{name}\"{sel}>{name}</option>"));
    }
    let mut catalog = String::from("<ul>");
    for c in &cats {
        catalog.push_str(&format!("<li>{} {}</li>", c.mid, html_escape(&c.name)));
    }
    catalog.push_str("</ul>");
    let check = |on: bool| if on { "checked" } else { "" };
    let body = format!(
        r#"<h1>编辑</h1>
<form method="post" action="{action}" class="stack">
<label>标题</label><input name="title" value="{title}" required>
<label>缩略名</label><input name="slug" value="{slug}">
<label>正文</label><textarea name="text" rows="16">{text}</textarea>
<label>状态</label><select name="status">{options}</select>
<label>发布时间（站点时区）</label><input name="created" value="{created_text}">
<label>密码</label><input name="password" value="{password}">
<label><input type="checkbox" name="allowComment" {}> 允许评论</label>
<label><input type="checkbox" name="allowPing" {}> 允许引用</label>
<label><input type="checkbox" name="allowFeed" {}> 允许订阅</label>
<label>分类编号，逗号分隔</label><input name="categories" value="{cat_text}">
{catalog}
<label>标签，逗号分隔</label><input name="tags" value="{tags}">
<button type="submit">保存</button>
</form>"#,
        check(existing.as_ref().map(|p| p.allow_comment).unwrap_or(true)),
        check(existing.as_ref().map(|p| p.allow_ping).unwrap_or(true)),
        check(existing.as_ref().map(|p| p.allow_feed).unwrap_or(true)),
        title = html_escape(title),
        slug = html_escape(slug),
        text = html_escape(text),
        password = html_escape(password),
        cat_text = html_escape(&cat_text),
        tags = html_escape(&tag_text),
    );
    page(&site, &user, "编辑", &body)
}

async fn content_save_new(State(state): State<Arc<AppState>>, headers: HeaderMap, body: String) -> Response {
    save_kind(&state, &headers, "post", None, &body).await
}
async fn page_save_new(State(state): State<Arc<AppState>>, headers: HeaderMap, body: String) -> Response {
    save_kind(&state, &headers, "page", None, &body).await
}
async fn content_save(State(state): State<Arc<AppState>>, headers: HeaderMap, Path(cid): Path<i64>, body: String) -> Response {
    save_kind(&state, &headers, "post", Some(cid), &body).await
}
async fn page_save(State(state): State<Arc<AppState>>, headers: HeaderMap, Path(cid): Path<i64>, body: String) -> Response {
    save_kind(&state, &headers, "page", Some(cid), &body).await
}

async fn save_kind(state: &AppState, headers: &HeaderMap, kind: &str, cid: Option<i64>, body: &str) -> Response {
    let Ok((site, user, _)) = need(state, headers, "contributor").await else {
        return redirect("/admin/login");
    };
    if kind == "page" && !user.pass("editor") {
        return plain(StatusCode::FORBIDDEN, "独立页需要编辑权限");
    }
    let form = pairs(body);
    let existing = if let Some(cid) = cid {
        match state.db.content_by_id(cid).await {
            Ok(Some(post)) if user.pass("editor") || post.author_id == user.uid => Some(post),
            Ok(Some(_)) => return plain(StatusCode::FORBIDDEN, "不能编辑这条内容"),
            _ => return plain(StatusCode::NOT_FOUND, "内容不存在"),
        }
    } else {
        None
    };
    let mut status = form.get("status").cloned().unwrap_or_else(|| "draft".into());
    if !user.pass("editor") && status == "publish" {
        status = "waiting".into();
    }
    let created = parse_local(form.get("created").map(String::as_str).unwrap_or(""), site.timezone())
        .unwrap_or_else(|| existing.as_ref().map(|p| p.created).unwrap_or_else(|| chrono::Utc::now().timestamp()));
    let categories = form
        .get("categories")
        .map(|s| s.split(',').filter_map(|n| n.trim().parse().ok()).collect())
        .unwrap_or_default();
    let tags = form
        .get("tags")
        .map(|s| s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect())
        .unwrap_or_default();
    let saved = state
        .db
        .save_content(&ContentInput {
            cid: existing.as_ref().map(|p| p.cid),
            title: form.get("title").cloned().unwrap_or_else(|| "未命名".into()),
            slug: form.get("slug").cloned().unwrap_or_default(),
            text: form.get("text").cloned().unwrap_or_default(),
            created,
            kind: kind.into(),
            status,
            password: form.get("password").cloned().unwrap_or_default(),
            allow_comment: form.contains_key("allowComment"),
            allow_ping: form.contains_key("allowPing"),
            allow_feed: form.contains_key("allowFeed"),
            author_id: existing.as_ref().map(|p| p.author_id).unwrap_or(user.uid),
            parent: 0,
            sort_order: existing.as_ref().map(|p| p.sort_order).unwrap_or(0),
            categories,
            tags,
        })
        .await;
    match saved {
        Ok(id) => {
            let base = if kind == "page" { "/admin/pages" } else { "/admin/posts" };
            redirect(&format!("{base}/{id}"))
        }
        Err(_) => plain(StatusCode::INTERNAL_SERVER_ERROR, "保存失败"),
    }
}

async fn content_delete(State(state): State<Arc<AppState>>, headers: HeaderMap, Path(cid): Path<i64>) -> Response {
    delete_kind(&state, &headers, cid, "/admin/posts").await
}
async fn page_delete(State(state): State<Arc<AppState>>, headers: HeaderMap, Path(cid): Path<i64>) -> Response {
    delete_kind(&state, &headers, cid, "/admin/pages").await
}

async fn delete_kind(state: &AppState, headers: &HeaderMap, cid: i64, back: &str) -> Response {
    let Ok((_, user, _)) = need(state, headers, "contributor").await else {
        return redirect("/admin/login");
    };
    let Ok(Some(post)) = state.db.content_by_id(cid).await else {
        return plain(StatusCode::NOT_FOUND, "内容不存在");
    };
    if !user.pass("editor") && post.author_id != user.uid {
        return plain(StatusCode::FORBIDDEN, "不能删除这条内容");
    }
    let _ = state.db.delete_content(cid).await;
    redirect(back)
}

async fn categories(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    meta_page(&state, &headers, "category", "分类", "/admin/categories").await
}
async fn tags(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    meta_page(&state, &headers, "tag", "标签", "/admin/tags").await
}

async fn meta_page(state: &AppState, headers: &HeaderMap, kind: &str, title: &str, base: &str) -> Response {
    let Ok((site, user, _)) = need(state, headers, "editor").await else {
        return redirect("/admin/login");
    };
    let rows = state.db.list_metas(kind).await.unwrap_or_default();
    let mut table = String::from("<table><tr><th>名称</th><th>缩略名</th><th>数量</th><th></th></tr>");
    for row in rows {
        table.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td><form method=\"post\" action=\"{base}/{}/delete\"><button>删除</button></form></td></tr>",
            html_escape(&row.name),
            html_escape(&row.slug),
            row.count,
            row.mid
        ));
    }
    table.push_str("</table>");
    let parent = if kind == "category" {
        "<label>父级编号</label><input name=\"parent\" value=\"0\">"
    } else {
        ""
    };
    let body = format!(
        r#"{table}<h2>新建</h2><form method="post" action="{base}" class="stack">
<label>名称</label><input name="name" required>
<label>缩略名</label><input name="slug">
<label>描述</label><input name="description">
{parent}
<button type="submit">保存</button></form>"#
    );
    page(&site, &user, title, &body)
}

async fn category_save(State(state): State<Arc<AppState>>, headers: HeaderMap, body: String) -> Response {
    meta_save(&state, &headers, "category", "/admin/categories", &body).await
}
async fn tag_save(State(state): State<Arc<AppState>>, headers: HeaderMap, body: String) -> Response {
    meta_save(&state, &headers, "tag", "/admin/tags", &body).await
}

async fn meta_save(state: &AppState, headers: &HeaderMap, kind: &str, back: &str, body: &str) -> Response {
    let Ok((_, _, _)) = need(state, headers, "editor").await else {
        return redirect("/admin/login");
    };
    let form = pairs(body);
    let _ = state
        .db
        .save_meta(&MetaInput {
            mid: None,
            name: form.get("name").cloned().unwrap_or_default(),
            slug: form.get("slug").cloned().unwrap_or_default(),
            kind: kind.into(),
            description: form.get("description").cloned().unwrap_or_default(),
            parent: form.get("parent").and_then(|s| s.parse().ok()).unwrap_or(0),
        })
        .await;
    redirect(back)
}

async fn category_delete(State(state): State<Arc<AppState>>, headers: HeaderMap, Path(mid): Path<i64>) -> Response {
    meta_delete(&state, &headers, mid, "/admin/categories").await
}
async fn tag_delete(State(state): State<Arc<AppState>>, headers: HeaderMap, Path(mid): Path<i64>) -> Response {
    meta_delete(&state, &headers, mid, "/admin/tags").await
}

async fn meta_delete(state: &AppState, headers: &HeaderMap, mid: i64, back: &str) -> Response {
    let Ok((_, _, _)) = need(state, headers, "editor").await else {
        return redirect("/admin/login");
    };
    let _ = state.db.delete_meta(mid).await;
    redirect(back)
}

async fn comments(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let Ok((site, user, _)) = need(&state, &headers, "editor").await else {
        return redirect("/admin/login");
    };
    let rows = state.db.list_comments(100).await.unwrap_or_default();
    let mut table = String::from("<table><tr><th>作者</th><th>内容</th><th>状态</th><th>操作</th></tr>");
    for row in rows {
        table.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            html_escape(&row.author),
            html_escape(&crate::text::truncate_chars(&row.text, 80)),
            html_escape(&row.status),
            comment_actions(row.coid)
        ));
    }
    table.push_str("</table>");
    page(&site, &user, "评论", &table)
}

fn comment_actions(coid: i64) -> String {
    ["approved", "spam", "delete"]
        .iter()
        .map(|act| {
            format!(
                "<form method=\"post\" action=\"/admin/comments/{coid}\" style=\"display:inline\"><input type=\"hidden\" name=\"act\" value=\"{act}\"><button>{act}</button></form>"
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

async fn comment_act(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(coid): Path<i64>,
    body: String,
) -> Response {
    let Ok((_, _, _)) = need(&state, &headers, "editor").await else {
        return redirect("/admin/login");
    };
    let form = pairs(&body);
    let act = form.get("act").map(String::as_str).unwrap_or("");
    if act == "delete" {
        let _ = state.db.delete_comment(coid).await;
    } else if act == "approved" || act == "spam" {
        let _ = state.db.set_comment_status(coid, act).await;
    }
    redirect("/admin/comments")
}

async fn media(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let Ok((site, user, _)) = need(&state, &headers, "contributor").await else {
        return redirect("/admin/login");
    };
    let rows = state.db.list_contents("attachment", 100).await.unwrap_or_default();
    let mut table = String::from("<table><tr><th>文件</th><th></th></tr>");
    for row in rows {
        if !user.pass("editor") && row.author_id != user.uid {
            continue;
        }
        table.push_str(&format!(
            "<tr><td><a href=\"/attachment/{}/\">{}</a></td><td><form method=\"post\" action=\"/admin/media/{}/delete\"><button>删除</button></form></td></tr>",
            row.cid,
            html_escape(&row.title),
            row.cid
        ));
    }
    table.push_str("</table>");
    let body = format!(
        r#"{table}<h2>上传</h2><form method="post" action="/admin/media" enctype="multipart/form-data" class="stack">
<label>文件</label><input type="file" name="file" required>
<label>挂到文章编号（可空）</label><input name="parent">
<button type="submit">上传</button></form>"#
    );
    page(&site, &user, "附件", &body)
}

async fn media_upload(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let Ok((site, user, _)) = need(&state, &headers, "contributor").await else {
        return redirect("/admin/login");
    };
    let mut parent = 0i64;
    let mut name = String::from("file.bin");
    let mut bytes = Vec::new();
    while let Ok(Some(field)) = multipart.next_field().await {
        let field_name = field.name().unwrap_or("").to_string();
        if field_name == "parent" {
            parent = field.text().await.unwrap_or_default().trim().parse().unwrap_or(0);
        } else if field_name == "file" {
            if let Some(n) = field.file_name() {
                name = n.to_string();
            }
            bytes = field.bytes().await.unwrap_or_default().to_vec();
        }
    }
    if bytes.is_empty() {
        return plain(StatusCode::BAD_REQUEST, "没有文件");
    }
    let safe = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("file.bin")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect::<String>();
    let safe = if safe.is_empty() { "file.bin".into() } else { safe };
    let ext = safe.rsplit('.').next().unwrap_or("bin").to_ascii_lowercase();
    let now = chrono::Utc::now().timestamp();
    let local = now + site.timezone();
    let stamp = chrono::DateTime::from_timestamp(local, 0);
    let folder = stamp
        .map(|d| format!("{}/{}", d.format("%Y"), d.format("%m")))
        .unwrap_or_else(|| "0000/00".into());
    let file_name = format!("{}-{safe}", &auth::md5_hex(&bytes)[..12]);
    let root = state
        .cfg
        .uploads_dir
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("usr/uploads"));
    let dir = root.join(&folder);
    if std::fs::create_dir_all(&dir).is_err() || std::fs::write(dir.join(&file_name), &bytes).is_err() {
        return plain(StatusCode::INTERNAL_SERVER_ERROR, "写入上传目录失败");
    }
    let rel = format!("usr/uploads/{folder}/{file_name}");
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    };
    let text = crate::write::attachment_text(&safe, &rel, bytes.len() as i64, &ext, mime);
    let _ = state
        .db
        .save_content(&ContentInput {
            cid: None,
            title: safe,
            slug: String::new(),
            text,
            created: now,
            kind: "attachment".into(),
            status: "publish".into(),
            password: String::new(),
            allow_comment: false,
            allow_ping: false,
            allow_feed: false,
            author_id: user.uid,
            parent,
            sort_order: 0,
            categories: Vec::new(),
            tags: Vec::new(),
        })
        .await;
    redirect("/admin/media")
}

async fn media_delete(State(state): State<Arc<AppState>>, headers: HeaderMap, Path(cid): Path<i64>) -> Response {
    delete_kind(&state, &headers, cid, "/admin/media").await
}

async fn users(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let Ok((site, user, _)) = need(&state, &headers, "administrator").await else {
        return redirect("/admin/login");
    };
    let rows = state.db.list_users().await.unwrap_or_default();
    let mut table = String::from("<table><tr><th>用户</th><th>组</th><th>邮箱</th><th></th></tr>");
    for row in &rows {
        table.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td><form method=\"post\" action=\"/admin/users/{}/delete\"><button>删除</button></form></td></tr>",
            html_escape(&row.name),
            html_escape(&row.group),
            html_escape(&row.mail),
            row.uid
        ));
    }
    table.push_str("</table>");
    let body = format!(
        r#"{table}<h2>新建或更新</h2>
<form method="post" action="/admin/users" class="stack">
<label>编号（留空为新建）</label><input name="uid">
<label>用户名</label><input name="name" required>
<label>显示名</label><input name="screen">
<label>邮箱</label><input name="mail">
<label>网址</label><input name="url">
<label>密码（更新时留空则不改）</label><input name="password" type="password">
<label>用户组</label><select name="group">
<option>administrator</option><option>editor</option><option>contributor</option><option selected>subscriber</option>
</select>
<button type="submit">保存</button></form>"#
    );
    page(&site, &user, "用户", &body)
}

async fn user_save(State(state): State<Arc<AppState>>, headers: HeaderMap, body: String) -> Response {
    let Ok((_, _, _)) = need(&state, &headers, "administrator").await else {
        return redirect("/admin/login");
    };
    let form = pairs(&body);
    let uid = form.get("uid").and_then(|s| s.parse().ok()).filter(|n: &i64| *n > 0);
    let now = chrono::Utc::now().timestamp();
    let _ = state
        .db
        .save_user(&UserInput {
            uid,
            name: form.get("name").cloned().unwrap_or_default(),
            password: form.get("password").cloned().unwrap_or_default(),
            mail: form.get("mail").cloned().unwrap_or_default(),
            url: form.get("url").cloned().unwrap_or_default(),
            screen: form
                .get("screen")
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| form.get("name").cloned().unwrap_or_default()),
            group: form.get("group").cloned().unwrap_or_else(|| "subscriber".into()),
            created: now,
        })
        .await;
    redirect("/admin/users")
}

async fn user_delete(State(state): State<Arc<AppState>>, headers: HeaderMap, Path(uid): Path<i64>) -> Response {
    let Ok((_, user, _)) = need(&state, &headers, "administrator").await else {
        return redirect("/admin/login");
    };
    if uid == user.uid {
        return plain(StatusCode::BAD_REQUEST, "不能删除自己");
    }
    if let Ok(rows) = state.db.list_users().await {
        let target = rows.iter().find(|r| r.uid == uid);
        let admins = rows.iter().filter(|r| r.group == "administrator").count();
        if target.is_some_and(|r| r.group == "administrator") && admins <= 1 {
            return plain(StatusCode::BAD_REQUEST, "至少保留一位管理员");
        }
    }
    let _ = state.db.delete_user(uid).await;
    redirect("/admin/users")
}

const SETTINGS: &[&str] = &[
    "title",
    "description",
    "keywords",
    "siteUrl",
    "rewrite",
    "pageSize",
    "timezone",
    "feedFullText",
    "commentsRequireMail",
    "commentsRequireURL",
    "commentsRequireModeration",
    "commentsWhitelist",
    "commentsCheckReferer",
    "commentsPostIntervalEnable",
    "commentsPostInterval",
    "commentsMarkdown",
    "commentsThreaded",
    "commentsPageBreak",
    "commentsPageSize",
    "commentsPageDisplay",
    "commentsOrder",
    "commentsShowCommentOnly",
    "commentsAutoClose",
    "commentsPostTimeout",
    "allowXmlRpc",
    "allowRegister",
    "xmlrpcMarkdown",
    "defaultAllowComment",
    "defaultAllowPing",
    "defaultAllowFeed",
];

async fn settings(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let Ok((site, user, _)) = need(&state, &headers, "administrator").await else {
        return redirect("/admin/login");
    };
    let mut form = String::from("<form method=\"post\" action=\"/admin/settings\" class=\"stack\">");
    for key in SETTINGS {
        form.push_str(&format!(
            "<label>{key}</label><input name=\"{key}\" value=\"{}\">",
            html_escape(site.get(key))
        ));
    }
    form.push_str(&format!(
        "<label>routingTable</label><textarea name=\"routingTable\" rows=\"8\">{}</textarea>",
        html_escape(site.get("routingTable"))
    ));
    form.push_str(&format!(
        "<label>commentsStopWords</label><textarea name=\"commentsStopWords\" rows=\"4\">{}</textarea>",
        html_escape(site.get("commentsStopWords"))
    ));
    form.push_str(&format!(
        "<label>commentsIpBlackList</label><textarea name=\"commentsIpBlackList\" rows=\"4\">{}</textarea>",
        html_escape(site.get("commentsIpBlackList"))
    ));
    form.push_str("<button type=\"submit\">保存</button></form>");
    page(&site, &user, "设置", &form)
}

async fn settings_save(State(state): State<Arc<AppState>>, headers: HeaderMap, body: String) -> Response {
    let Ok((_, _, _)) = need(&state, &headers, "administrator").await else {
        return redirect("/admin/login");
    };
    let form = pairs(&body);
    let mut keys: Vec<&str> = SETTINGS.to_vec();
    keys.extend(["routingTable", "commentsStopWords", "commentsIpBlackList"]);
    for key in keys {
        if let Some(value) = form.get(key) {
            let _ = state.db.set_option(key, value).await;
        }
    }
    redirect("/admin/settings")
}

async fn need(
    state: &AppState,
    headers: &HeaderMap,
    group: &str,
) -> Result<(Site, SessionUser, String), ()> {
    let site = load_site(state).await.map_err(|_| ())?;
    let root = auth::site_root(&state.cfg.site_url, &links::site_base(&state.cfg.site_url, &site));
    let cookies = cookies_from(headers);
    let user = auth::session_user(&state.db, &root, &cookies).await.map_err(|_| ())?;
    match user {
        Some(user) if user.pass(group) => Ok((site, user, root)),
        _ => Err(()),
    }
}

async fn session(state: &AppState, headers: &HeaderMap) -> Result<Option<SessionUser>, sqlx::Error> {
    let root = root_of(state, headers).await;
    let cookies = cookies_from(headers);
    auth::session_user(&state.db, &root, &cookies).await
}

async fn root_of(state: &AppState, _headers: &HeaderMap) -> String {
    if let Ok(site) = load_site(state).await {
        auth::site_root(&state.cfg.site_url, &links::site_base(&state.cfg.site_url, &site))
    } else {
        auth::site_root(&state.cfg.site_url, "")
    }
}

async fn load_site(state: &AppState) -> Result<Site, sqlx::Error> {
    state.db.load_site(chrono::Utc::now().timestamp()).await
}

fn nav_items(user: &SessionUser) -> String {
    let mut items = vec![
        "<li><a href=\"/admin/posts\">文章</a></li>",
        "<li><a href=\"/admin/media\">附件</a></li>",
    ];
    if user.pass("editor") {
        items.push("<li><a href=\"/admin/pages\">独立页</a></li>");
        items.push("<li><a href=\"/admin/categories\">分类</a></li>");
        items.push("<li><a href=\"/admin/tags\">标签</a></li>");
        items.push("<li><a href=\"/admin/comments\">评论</a></li>");
    }
    if user.pass("administrator") {
        items.push("<li><a href=\"/admin/users\">用户</a></li>");
        items.push("<li><a href=\"/admin/settings\">设置</a></li>");
    }
    items.push("<li><a href=\"/\">查看站点</a></li>");
    items.push("<li><form method=\"post\" action=\"/admin/logout\"><button>退出</button></form></li>");
    items.join("")
}

fn page(site: &Site, user: &SessionUser, title: &str, body: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><title>{} · {}</title>{STYLE}</head><body><header><strong>{}</strong> <a href=\"/admin\">后台</a></header><main><nav><ul>{}</ul></nav>{body}</main></body></html>",
        html_escape(title),
        html_escape(site.get_or("title", "Tiebidli")),
        html_escape(&user.label()),
        nav_items(user)
    );
    html_ok(html)
}

fn bare(title: &str, body: &str) -> Response {
    html_ok(format!(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><title>{}</title>{STYLE}</head><body><main>{body}</main></body></html>",
        html_escape(title)
    ))
}

const STYLE: &str = "<style>body{font:15px/1.5 sans-serif;margin:0;background:#f4f5f7;color:#222}header{background:#1f2933;color:#fff;padding:12px 20px}header a{color:#fff}main{max-width:980px;margin:24px auto;padding:0 16px}table{width:100%;border-collapse:collapse;background:#fff}td,th{border-bottom:1px solid #e5e7eb;padding:8px;text-align:left;vertical-align:top}form.stack label{display:block;margin:12px 0 4px}input,textarea,select{width:100%;max-width:720px;padding:8px;border:1px solid #ccc;box-sizing:border-box}input[type=checkbox]{width:auto;padding:0;border:0;vertical-align:middle}button{background:#2563eb;color:#fff;border:0;padding:8px 14px;cursor:pointer}nav ul{display:flex;gap:12px;list-style:none;padding:0}.warn{color:#b91c1c}</style>";

fn html_ok(body: String) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(body))
        .unwrap_or_else(|_| plain(StatusCode::INTERNAL_SERVER_ERROR, "页面失败"))
}

fn plain(status: StatusCode, msg: &str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(msg.to_string()))
        .unwrap_or_else(|_| Response::new(Body::from(msg.to_string())))
}

fn redirect(loc: &str) -> Response {
    redirect_with(loc, &HeaderMap::new())
}

fn redirect_with(loc: &str, extra: &HeaderMap) -> Response {
    let mut res = Response::builder()
        .status(StatusCode::FOUND)
        .header(header::LOCATION, loc)
        .body(Body::from("redirect"))
        .unwrap_or_else(|_| plain(StatusCode::FOUND, loc));
    for (k, v) in extra.iter() {
        res.headers_mut().append(k, v.clone());
    }
    res
}

fn pairs(raw: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let k = decode(k);
        if !k.is_empty() {
            map.insert(k, decode(v));
        }
    }
    map
}

fn decode(s: &str) -> String {
    let spaced = s.replace('+', " ");
    urlencoding::decode(&spaced).map(|c| c.into_owned()).unwrap_or(spaced)
}

fn cookies_from(headers: &HeaderMap) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for val in headers.get_all(header::COOKIE) {
        let Ok(s) = val.to_str() else { continue };
        for part in s.split(';') {
            let Some((k, v)) = part.trim().split_once('=') else { continue };
            map.insert(k.to_string(), decode(v));
        }
    }
    map
}

fn push_cookie(headers: &mut HeaderMap, name: &str, value: &str, max_age: Option<i64>) {
    let enc = urlencoding::encode(value);
    let mut line = format!("{name}={enc}; Path=/; SameSite=Lax");
    if let Some(age) = max_age {
        line.push_str(&format!("; Max-Age={age}"));
    }
    if let Ok(v) = HeaderValue::from_str(&line) {
        headers.append(header::SET_COOKIE, v);
    }
}

fn parse_local(raw: &str, tz: i64) -> Option<i64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(n) = raw.parse::<i64>() {
        return Some(n);
    }
    for fmt in ["%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(raw, fmt) {
            return Some(dt.and_utc().timestamp() - tz);
        }
    }
    None
}
