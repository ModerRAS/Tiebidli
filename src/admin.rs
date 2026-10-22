//! 后台页面按 Typecho 1.2.1 的 `admin/` 布局和操作来。
//! 样式来自同版本的 `admin/css` 与 `admin/img`。注册只在 `allowRegister` 打开时可用。

use crate::auth::{self, SessionUser};
use crate::http::AppState;
use crate::links;
use crate::model::{Post, Site};
use crate::phpval::{self, PhpValue};
use crate::text::html_escape;
use crate::write::{ContentInput, MetaInput, UserInput};
use axum::body::Body;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use std::collections::HashMap;
use std::sync::Arc;

const PAGE: usize = 20;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/admin", get(dashboard))
        .route("/admin/login", get(login_form).post(login_submit))
        .route("/admin/logout", get(logout).post(logout))
        .route("/admin/register", get(register_form).post(register_submit))
        .route("/admin/posts", get(content_list))
        .route("/admin/posts/new", get(content_new).post(content_save_new))
        .route("/admin/posts/batch", post(posts_batch))
        .route("/admin/posts/:cid", get(content_edit).post(content_save))
        .route("/admin/posts/:cid/delete", post(content_delete))
        .route("/admin/pages", get(page_list))
        .route("/admin/pages/new", get(page_new).post(page_save_new))
        .route("/admin/pages/batch", post(pages_batch))
        .route("/admin/pages/:cid", get(page_edit).post(page_save))
        .route("/admin/pages/:cid/delete", post(page_delete))
        .route("/admin/categories", get(categories).post(category_save))
        .route("/admin/categories/batch", post(categories_batch))
        .route("/admin/categories/:mid/delete", post(category_delete))
        .route("/admin/tags", get(tags).post(tag_save))
        .route("/admin/tags/batch", post(tags_batch))
        .route("/admin/tags/:mid/delete", post(tag_delete))
        .route("/admin/comments", get(comments))
        .route("/admin/comments/batch", post(comments_batch))
        .route("/admin/comments/act", get(comment_get_act))
        .route("/admin/comments/:coid", post(comment_act))
        .route("/admin/media", get(media).post(media_upload))
        .route("/admin/media/batch", post(media_batch))
        .route("/admin/media/:cid/delete", post(media_delete))
        .route("/admin/users", get(users).post(user_save))
        .route("/admin/users/batch", post(users_batch))
        .route("/admin/users/:uid/delete", post(user_delete))
        .route("/admin/settings", get(settings_general).post(settings_save_general))
        .route("/admin/settings/general", get(settings_general).post(settings_save_general))
        .route("/admin/settings/discussion", get(settings_discussion).post(settings_save_discussion))
        .route("/admin/settings/reading", get(settings_reading).post(settings_save_reading))
        .route("/admin/settings/permalink", get(settings_permalink).post(settings_save_permalink))
}

async fn dashboard(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let Ok((site, user, _)) = need(&state, &headers, "subscriber").await else {
        return redirect("/admin/login");
    };
    let posts = state.db.list_contents("post", 200).await.unwrap_or_default();
    let comments = state.db.list_comments(200).await.unwrap_or_default();
    let mine: Vec<&Post> = posts.iter().filter(|p| p.author_id == user.uid).collect();
    let published = mine.iter().filter(|p| p.status == "publish").count();
    let my_comments = comments.iter().filter(|c| c.owner_id == user.uid && c.status == "approved").count();
    let waiting = comments.iter().filter(|c| c.status == "waiting").count();
    let cats = site.categories.len();
    let mut start = String::new();
    if user.pass("contributor") {
        start.push_str("<li><a href=\"/admin/posts/new\">撰写新文章</a></li>");
    }
    if waiting > 0 && user.pass("editor") {
        start.push_str(&format!(
            "<li><a href=\"/admin/comments?status=waiting\">待审核的评论</a><span class=\"balloon\">{waiting}</span></li>"
        ));
    }
    if user.pass("administrator") {
        start.push_str("<li><a href=\"/admin/settings/general\">系统设置</a></li>");
    }
    let mut recent_posts = String::new();
    let shown: Vec<&Post> = mine.iter().filter(|p| p.status == "publish").take(10).copied().collect();
    if shown.is_empty() {
        recent_posts.push_str("<li><em>暂时没有文章</em></li>");
    } else {
        for post in shown {
            recent_posts.push_str(&format!(
                "<li><span>{}</span> <a href=\"{}\" class=\"title\">{}</a></li>",
                date_short(post.created, site.timezone()),
                html_escape(&links::content_path(&site, post)),
                html_escape(&post.title)
            ));
        }
    }
    let mut recent_comments = String::new();
    let mine_comments: Vec<_> = comments.iter().filter(|c| c.owner_id == user.uid).take(10).collect();
    if mine_comments.is_empty() {
        recent_comments.push_str("<li>暂时没有回复</li>");
    } else {
        for c in mine_comments {
            recent_comments.push_str(&format!(
                "<li><span>{}</span> <a class=\"title\" href=\"/admin/comments?cid={}\">{}</a>: {}</li>",
                date_short(c.created, site.timezone()),
                c.cid,
                html_escape(&c.author),
                html_escape(&crate::text::truncate_chars(&c.text, 35))
            ));
        }
    }
    let body = format!(
        r#"<div class="row typecho-page-main">
<div class="col-mb-12 welcome-board" role="main">
<p>目前有 <em>{published}</em> 篇文章, 并有 <em>{my_comments}</em> 条关于你的评论在 <em>{cats}</em> 个分类中.<br>点击下面的链接快速开始:</p>
<ul id="start-link" class="clearfix">{start}</ul>
</div>
<div class="col-mb-12 col-tb-4" role="complementary"><section class="latest-link"><h3>最近发布的文章</h3><ul>{recent_posts}</ul></section></div>
<div class="col-mb-12 col-tb-4" role="complementary"><section class="latest-link"><h3>最近得到的回复</h3><ul>{recent_comments}</ul></section></div>
<div class="col-mb-12 col-tb-4" role="complementary"><section class="latest-link"><h3>官方最新日志</h3><div id="typecho-message"><ul><li><em>这里不拉取外部日志</em></li></ul></div></section></div>
</div>"#
    );
    layout(&site, &user, "dashboard", "概要", None, "container typecho-dashboard", &body)
}

async fn login_form(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if session(&state, &headers).await.ok().flatten().is_some() {
        return redirect("/admin");
    }
    let Ok(site) = load_site(&state).await else {
        return plain(StatusCode::INTERNAL_SERVER_ERROR, "读取设置失败");
    };
    let error = q.get("error").map(|s| !s.is_empty()).unwrap_or(false);
    let referer = q.get("referer").map(|s| s.as_str()).unwrap_or("");
    let register = if site.flag("allowRegister", false) {
        " &bull;\n<a href=\"/admin/register\">用户注册</a>"
    } else {
        ""
    };
    let message = if error {
        "<div class=\"message error\">用户名或密码无效</div>"
    } else {
        ""
    };
    let html = format!(
        r#"<div class="typecho-login-wrap"><div class="typecho-login">
<h1><a href="https://typecho.org" class="i-logo">Typecho</a></h1>
{message}
<form action="/admin/login" method="post" name="login" role="form">
<p><label for="name" class="sr-only">用户名</label>
<input type="text" id="name" name="name" placeholder="用户名" class="text-l w-100" autofocus></p>
<p><label for="password" class="sr-only">密码</label>
<input type="password" id="password" name="password" class="text-l w-100" placeholder="密码"></p>
<p class="submit"><button type="submit" class="btn btn-l w-100 primary">登录</button>
<input type="hidden" name="referer" value="{referer}"></p>
<p><label for="remember"><input type="checkbox" name="remember" class="checkbox" value="1" id="remember"> 下次自动登录</label></p>
</form>
<p class="more-link"><a href="/">返回首页</a>{register}</p>
</div></div>"#,
        referer = html_escape(referer),
    );
    guest(&site, "登录", "body-100", &html)
}

async fn login_submit(State(state): State<Arc<AppState>>, headers: HeaderMap, body: String) -> Response {
    let form = pairs(&body);
    let name = form.get("name").map(|s| s.as_str()).unwrap_or("");
    let password = form.get("password").map(|s| s.as_str()).unwrap_or("");
    let next = safe_next(form.get("referer").map(|s| s.as_str()).unwrap_or(""));
    let Ok(Some(user)) = auth::login_user(&state.db, name, password).await else {
        return redirect("/admin/login?error=1");
    };
    let Ok(code) = auth::commit_login(&state.db, user.uid).await else {
        return plain(StatusCode::INTERNAL_SERVER_ERROR, "登录失败");
    };
    let root = root_of(&state, &headers).await;
    let age = if form.contains_key("remember") { Some(30 * 24 * 3600) } else { None };
    let mut extra = HeaderMap::new();
    push_cookie(&mut extra, &auth::cookie_name(&root, "__typecho_uid"), &user.uid.to_string(), age);
    push_cookie(&mut extra, &auth::cookie_name(&root, "__typecho_authCode"), &code, age);
    redirect_with(next, &extra)
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

async fn register_form(State(state): State<Arc<AppState>>, _headers: HeaderMap) -> Response {
    let Ok(site) = load_site(&state).await else {
        return plain(StatusCode::INTERNAL_SERVER_ERROR, "读取设置失败");
    };
    if !site.flag("allowRegister", false) {
        return plain(StatusCode::NOT_FOUND, "未开放注册");
    }
    guest(
        &site,
        "注册",
        "body-100",
        r#"<div class="typecho-login-wrap"><div class="typecho-login">
<h1><a href="https://typecho.org" class="i-logo">Typecho</a></h1>
<form method="post" action="/admin/register" role="form">
<p><input class="text-l w-100" name="name" placeholder="用户名" required></p>
<p><input class="text-l w-100" name="mail" type="email" placeholder="邮箱" required></p>
<p><input class="text-l w-100" name="password" type="password" placeholder="密码" required></p>
<p class="submit"><button type="submit" class="btn btn-l w-100 primary">注册</button></p>
</form>
<p class="more-link"><a href="/admin/login">返回登录</a></p>
</div></div>"#,
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
    let now = chrono::Utc::now().timestamp();
    match state
        .db
        .save_user(&UserInput {
            uid: None,
            name: form.get("name").cloned().unwrap_or_default(),
            password: form.get("password").cloned().unwrap_or_default(),
            mail: form.get("mail").cloned().unwrap_or_default(),
            url: String::new(),
            screen: form.get("name").cloned().unwrap_or_default(),
            group: "subscriber".into(),
            created: now,
        })
        .await
    {
        Ok(_) => guest(
            &site,
            "注册",
            "body-100",
            r#"<div class="typecho-login-wrap"><div class="typecho-login"><p class="message success">注册成功，请<a href="/admin/login">登录</a></p></div></div>"#,
        ),
        Err(_) => guest(
            &site,
            "注册",
            "body-100",
            r#"<div class="typecho-login-wrap"><div class="typecho-login"><p class="message error">注册失败，用户名或邮箱可能已存在</p><p class="more-link"><a href="/admin/register">返回</a></p></div></div>"#,
        ),
    }
}

async fn content_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    list_kind(&state, &headers, "post", &q).await
}
async fn page_list(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    list_kind(&state, &headers, "page", &q).await
}

async fn list_kind(state: &AppState, headers: &HeaderMap, kind: &str, q: &HashMap<String, String>) -> Response {
    let need_rank = if kind == "page" { "editor" } else { "contributor" };
    let Ok((site, user, _)) = need(state, headers, need_rank).await else {
        return redirect("/admin/login");
    };
    let Ok(rows) = state.db.list_contents(kind, 500).await else {
        return plain(StatusCode::INTERNAL_SERVER_ERROR, "读取失败");
    };
    let status = q.get("status").map(|s| s.as_str()).unwrap_or("");
    let keywords = q.get("keywords").map(|s| s.as_str()).unwrap_or("");
    let category = q.get("category").and_then(|s| s.parse::<i64>().ok());
    let uid = q.get("uid").and_then(|s| s.parse::<i64>().ok());
    let all = q.get("__typecho_all_posts").map(|s| s == "on").unwrap_or(false);
    let filtered: Vec<&Post> = rows
        .iter()
        .filter(|p| status_tab(status, &p.status))
        .filter(|p| author_visible(&user, all, uid, p.author_id))
        .filter(|p| category.is_none_or(|mid| p.categories.iter().any(|c| c.mid == mid)))
        .filter(|p| keywords.is_empty() || title_hit(&p.title, keywords))
        .collect();
    let page = q.get("page").and_then(|s| s.parse::<usize>().ok()).unwrap_or(1).max(1);
    let pages = filtered.len().div_ceil(PAGE).max(1);
    let page = page.min(pages);
    let slice = &filtered[(page - 1) * PAGE..filtered.len().min(page * PAGE)];
    let base = if kind == "page" { "/admin/pages" } else { "/admin/posts" };
    let write = if kind == "page" { "/admin/pages/new" } else { "/admin/posts/new" };
    let title = if kind == "page" { "管理独立页面" } else { "管理文章" };
    let key = if kind == "page" { "pages" } else { "posts" };
    let waiting = rows.iter().filter(|p| p.status == "waiting" && author_visible(&user, all, uid, p.author_id)).count();
    let drafts = rows.iter().filter(|p| p.status == "draft" && author_visible(&user, all, uid, p.author_id)).count();
    let mut tabs = String::new();
    for (name, label, n) in [("" , "可用", 0usize), ("waiting", "待审核", waiting), ("draft", "草稿", drafts)] {
        if kind == "page" && name == "waiting" {
            continue;
        }
        let current = if status == name { " class=\"current\"" } else { "" };
        let balloon = if n > 0 { format!(" <span class=\"balloon\">{n}</span>") } else { String::new() };
        tabs.push_str(&format!(
            "<li{current}><a href=\"{base}?status={name}\">{label}{balloon}</a></li>"
        ));
    }
    let mut scope = String::new();
    if user.pass("editor") && kind == "post" {
        let mine = if all { "" } else { " class=\"current\"" };
        let every = if all { " class=\"current\"" } else { "" };
        scope = format!(
            "<ul class=\"typecho-option-tabs right\"><li{every}><a href=\"{base}?__typecho_all_posts=on&status={status}\">所有</a></li><li{mine}><a href=\"{base}?__typecho_all_posts=off&status={status}\">我的</a></li></ul>"
        );
    }
    let mut marks = String::new();
    if user.pass("editor") {
        for (st, label) in [("publish", "公开"), ("waiting", "待审核"), ("hidden", "隐藏"), ("private", "私密")] {
            marks.push_str(&format!(
                "<li><a href=\"{base}/batch?do=mark&status={st}\">标记为<strong>{label}</strong></a></li>"
            ));
        }
    }
    let mut cats = String::from("<option value=\"\">所有分类</option>");
    if kind == "post" {
        for c in &site.categories {
            let sel = if category == Some(c.mid) { " selected" } else { "" };
            cats.push_str(&format!("<option value=\"{}\"{sel}>{}</option>", c.mid, html_escape(&c.name)));
        }
    }
    let search = if kind == "post" {
        format!(
            r#"<input type="text" class="text-s" placeholder="请输入关键字" name="keywords" value="{kw}">
<select name="category">{cats}</select>"#,
            kw = html_escape(keywords)
        )
    } else {
        format!(
            r#"<input type="text" class="text-s" placeholder="请输入关键字" name="keywords" value="{}">"#,
            html_escape(keywords)
        )
    };
    let hidden_status = if status.is_empty() {
        String::new()
    } else {
        format!("<input type=\"hidden\" name=\"status\" value=\"{status}\">")
    };
    let mut body_rows = String::new();
    if slice.is_empty() {
        let empty = if kind == "page" { "没有任何页面" } else { "没有任何文章" };
        body_rows.push_str(&format!("<tr><td colspan=\"6\"><h6 class=\"typecho-list-table-title\">{empty}</h6></td></tr>"));
    }
    for post in slice {
        let badge = status_badge(post);
        let view = links::content_path(&site, post);
        body_rows.push_str(&format!(
            r#"<tr id="post-{cid}"><td class="kit-hidden-mb"><input type="checkbox" value="{cid}" name="cid[]"></td>
<td class="kit-hidden-mb"><a href="/admin/comments?cid={cid}" class="balloon-button" title="{n} 评论">{n}</a></td>
<td><a href="{base}/{cid}">{title}</a>{badge}
<a href="{base}/{cid}" title="编辑 {title}"><i class="i-edit"></i></a>
<a href="{view}" title="浏览 {title}"><i class="i-exlink"></i></a></td>
<td class="kit-hidden-mb"><a href="{base}?uid={uid}">{author}</a></td>
<td class="kit-hidden-mb">{cats}</td>
<td>{date}</td></tr>"#,
            cid = post.cid,
            n = post.comments_num,
            title = html_escape(&post.title),
            view = html_escape(&view),
            uid = post.author_id,
            author = html_escape(&post.display_author()),
            cats = post.categories.iter().map(|c| html_escape(&c.name)).collect::<Vec<_>>().join(", "),
            date = html_escape(&date_word(post.created, site.timezone())),
        ));
    }
    let pager = pager_html(page, pages, base, q);
    let cat_filter = search;
    let main = format!(
        r#"<div class="row typecho-page-main" role="main"><div class="col-mb-12 typecho-list">
<div class="clearfix">{scope}<ul class="typecho-option-tabs">{tabs}</ul></div>
<div class="typecho-list-operate clearfix"><form method="get">
<div class="operate"><label><i class="sr-only">全选</i><input type="checkbox" class="typecho-table-select-all"></label>
<div class="btn-group btn-drop"><button class="btn dropdown-toggle btn-s" type="button">选中项 <i class="i-caret-down"></i></button>
<ul class="dropdown-menu"><li><a lang="你确认要删除这些文章吗?" href="{base}/batch?do=delete">删除</a></li>{marks}</ul></div></div>
<div class="search" role="search">{cat_filter}<button type="submit" class="btn btn-s">筛选</button>{hidden_status}</div>
</form></div>
<form method="post" name="manage_posts" class="operate-form" action="{base}/batch">
<div class="typecho-table-wrap"><table class="typecho-list-table"><colgroup>
<col width="20" class="kit-hidden-mb"><col width="6%" class="kit-hidden-mb"><col width="45%"><col width="" class="kit-hidden-mb"><col width="18%" class="kit-hidden-mb"><col width="16%">
</colgroup><thead><tr><th class="kit-hidden-mb"></th><th class="kit-hidden-mb"></th><th>标题</th><th class="kit-hidden-mb">作者</th><th class="kit-hidden-mb">分类</th><th>日期</th></tr></thead>
<tbody>{body_rows}</tbody></table></div></form>
<div class="typecho-list-operate clearfix"><form method="get"><div class="operate"><label><i class="sr-only">全选</i><input type="checkbox" class="typecho-table-select-all"></label></div>{pager}</form></div>
</div></div>"#
    );
    layout(&site, &user, key, title, Some(write), "container", &main)
}

fn status_tab(filter: &str, status: &str) -> bool {
    match filter {
        "waiting" => status == "waiting",
        "draft" => status == "draft",
        _ => status != "waiting" && status != "draft",
    }
}

fn author_visible(user: &SessionUser, all: bool, uid: Option<i64>, author: i64) -> bool {
    if let Some(uid) = uid {
        return author == uid;
    }
    if !user.pass("editor") || !all {
        return author == user.uid;
    }
    true
}

fn title_hit(title: &str, keywords: &str) -> bool {
    let title = title.to_lowercase();
    keywords.split_whitespace().all(|w| title.contains(&w.to_lowercase()))
}

fn status_badge(post: &Post) -> String {
    let label = if post.status == "hidden" {
        "隐藏"
    } else if post.status == "waiting" {
        "待审核"
    } else if post.status == "private" {
        "私密"
    } else if post.status == "draft" {
        "草稿"
    } else if post.has_password() {
        "密码保护"
    } else {
        return String::new();
    };
    format!(" <em class=\"status\">{label}</em>")
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
    let need_rank = if kind == "page" { "editor" } else { "contributor" };
    let Ok((site, user, _)) = need(state, headers, need_rank).await else {
        return redirect("/admin/login");
    };
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
        ("page", Some(id)) => format!("/admin/pages/{id}"),
        ("page", None) => "/admin/pages/new".into(),
        (_, Some(id)) => format!("/admin/posts/{id}"),
        _ => "/admin/posts/new".into(),
    };
    let title = existing.as_ref().map(|p| p.title.as_str()).unwrap_or("");
    let slug = existing.as_ref().map(|p| p.slug.as_str()).unwrap_or("");
    let text = existing.as_ref().map(|p| p.text.as_str()).unwrap_or("");
    let status = existing.as_ref().map(|p| p.status.as_str()).unwrap_or("");
    let password = existing.as_ref().map(|p| p.password.as_str()).unwrap_or("");
    let created = existing.as_ref().map(|p| p.created).unwrap_or(0);
    let date_text = if created > 0 {
        date_word(created, site.timezone())
    } else {
        String::new()
    };
    let visibility = if !password.is_empty() && status == "publish" {
        "password"
    } else if status.is_empty() {
        "publish"
    } else {
        status
    };
    let pattern = site
        .routes
        .routes
        .iter()
        .find(|r| r.name == if kind == "page" { "page" } else { "post" })
        .map(|r| r.source.as_str())
        .unwrap_or(if kind == "page" { "/[slug].html" } else { "/archives/[cid:digital]/" });
    let mut permalink = decode_rule(pattern);
    if let Some(id) = cid {
        permalink = permalink.replace("{cid}", &id.to_string());
    }
    let slug_input = format!(
        "<input type=\"text\" id=\"slug\" name=\"slug\" autocomplete=\"off\" value=\"{}\" class=\"mono\">",
        html_escape(slug)
    );
    let slug_html = if permalink.contains("{slug}") {
        permalink.replace("{slug}", &slug_input)
    } else {
        format!("{permalink} {slug_input}")
    };
    let draft = if status == "draft" {
        "<cite class=\"edit-draft-notice\">当前正在编辑的是未发布的草稿</cite>"
    } else {
        ""
    };
    let mut vis = String::new();
    if user.pass("editor") {
        for (value, label) in [
            ("publish", "公开"),
            ("hidden", "隐藏"),
            ("password", "密码保护"),
            ("private", "私密"),
            ("waiting", "待审核"),
        ] {
            let sel = if visibility == value { " selected" } else { "" };
            vis.push_str(&format!("<option value=\"{value}\"{sel}>{label}</option>"));
        }
    }
    let pwd_hidden = if visibility == "password" { "" } else { " class=\"hidden\"" };
    let visibility_html = if user.pass("editor") {
        format!(
            r#"<section class="typecho-post-option visibility-option"><label for="visibility" class="typecho-label">公开度</label>
<p><select id="visibility" name="visibility">{vis}</select></p>
<p id="post-password"{pwd_hidden}><label for="protect-pwd" class="sr-only">内容密码</label>
<input type="text" name="password" id="protect-pwd" class="text-s" value="{password}" placeholder="内容密码" autocomplete="off"></p></section>"#,
            password = html_escape(password)
        )
    } else {
        String::new()
    };
    let mut cat_html = String::new();
    if kind == "post" {
        let selected: Vec<i64> = existing.as_ref().map(|p| p.categories.iter().map(|c| c.mid).collect()).unwrap_or_default();
        cat_html.push_str("<section class=\"typecho-post-option category-option\"><label class=\"typecho-label\">分类</label><ul>");
        for c in &site.categories {
            let checked = if selected.contains(&c.mid) { " checked" } else { "" };
            cat_html.push_str(&format!(
                "<li><input type=\"checkbox\" id=\"category-{mid}\" value=\"{mid}\" name=\"category[]\"{checked}><label for=\"category-{mid}\">{name}</label></li>",
                mid = c.mid,
                name = html_escape(&c.name)
            ));
        }
        cat_html.push_str("</ul></section>");
    }
    let tags = existing
        .as_ref()
        .map(|p| p.tags.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(","))
        .unwrap_or_default();
    let check = |on: bool| if on { " checked" } else { "" };
    let allow_comment = existing.as_ref().map(|p| p.allow_comment).unwrap_or(true);
    let allow_ping = existing.as_ref().map(|p| p.allow_ping).unwrap_or(true);
    let allow_feed = existing.as_ref().map(|p| p.allow_feed).unwrap_or(true);
    let publish_label = if kind == "page" { "发布页面" } else { "发布文章" };
    let page_title = match (&existing, kind) {
        (Some(p), "page") => format!("编辑 {}", p.title),
        (Some(p), _) => format!("编辑 {}", p.title),
        (None, "page") => "创建新页面".into(),
        _ => "撰写新文章".into(),
    };
    let key = if kind == "page" { "write-page" } else { "write-post" };
    let parent = cid.unwrap_or(0);
    let main = format!(
        r##"<div class="row typecho-page-main typecho-post-area" role="form">
<form action="{action}" method="post" name="write_post">
<div class="col-mb-12 col-tb-9" role="main">
{draft}
<p class="title"><label for="title" class="sr-only">标题</label>
<input type="text" id="title" name="title" autocomplete="off" value="{title}" placeholder="标题" class="w-100 text title"></p>
<p class="mono url-slug"><label for="slug" class="sr-only">网址缩略名</label>{slug_html}</p>
<p><label for="text" class="sr-only">文章内容</label>
<textarea style="height:350px" autocomplete="off" id="text" name="text" class="w-100 mono">{text}</textarea></p>
<p class="submit clearfix"><span class="right">
<button type="submit" name="do" value="save" id="btn-save" class="btn">保存草稿</button>
<button type="submit" name="do" value="publish" class="btn primary" id="btn-submit">{publish_label}</button>
</span></p>
</div>
<div id="edit-secondary" class="col-mb-12 col-tb-3" role="complementary">
<ul class="typecho-option-tabs clearfix">
<li class="active w-50"><a href="#tab-advance">选项</a></li>
<li class="w-50"><a href="#tab-files" id="tab-files-btn">附件</a></li>
</ul>
<div id="tab-advance" class="tab-content">
<section class="typecho-post-option"><label for="date" class="typecho-label">发布日期</label>
<p><input class="typecho-date w-100" type="text" name="date" id="date" autocomplete="off" value="{date}"></p></section>
{cat_html}
<section class="typecho-post-option"><label for="tags" class="typecho-label">标签</label>
<p><input id="tags" name="tags" type="text" value="{tags}" class="w-100 text"></p></section>
<button type="button" id="advance-panel-btn" class="btn btn-xs">高级选项 <i class="i-caret-down"></i></button>
<div id="advance-panel">
{visibility_html}
<section class="typecho-post-option allow-option"><label class="typecho-label">权限控制</label><ul>
<li><input id="allowComment" name="allowComment" type="checkbox" value="1"{c1}><label for="allowComment">允许评论</label></li>
<li><input id="allowPing" name="allowPing" type="checkbox" value="1"{c2}><label for="allowPing">允许被引用</label></li>
<li><input id="allowFeed" name="allowFeed" type="checkbox" value="1"{c3}><label for="allowFeed">允许在聚合中出现</label></li>
</ul></section>
</div>
</div>
<div id="tab-files" class="tab-content hidden">
<p class="description">上传的文件会出现在文件管理里。</p>
</div>
</div>
</form>
<form class="hidden" id="side-upload" method="post" action="/admin/media" enctype="multipart/form-data">
<input type="hidden" name="parent" value="{parent}">
<input type="file" name="file">
</form>
</div>"##,
        title = html_escape(title),
        text = html_escape(text),
        date = html_escape(&date_text),
        tags = html_escape(&tags),
        c1 = check(allow_comment),
        c2 = check(allow_ping),
        c3 = check(allow_feed),
    );
    layout(&site, &user, key, &page_title, None, "container", &main)
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
    let need_rank = if kind == "page" { "editor" } else { "contributor" };
    let Ok((site, user, _)) = need(state, headers, need_rank).await else {
        return redirect("/admin/login");
    };
    let form = pairs(body);
    let many = repeated(body);
    let existing = if let Some(cid) = cid {
        match state.db.content_by_id(cid).await {
            Ok(Some(post)) if user.pass("editor") || post.author_id == user.uid => Some(post),
            Ok(Some(_)) => return plain(StatusCode::FORBIDDEN, "不能编辑这条内容"),
            _ => return plain(StatusCode::NOT_FOUND, "内容不存在"),
        }
    } else {
        None
    };
    let (mut status, mut password) = resolve_status(&form, &user, existing.as_ref().map(|p| p.status.as_str()).unwrap_or("draft"));
    if password.is_empty() {
        if let Some(old) = existing.as_ref() {
            if form.get("visibility").is_none() && form.get("password").is_none() {
                password = old.password.clone();
            }
        }
    }
    if !user.pass("editor") && matches!(status.as_str(), "publish" | "hidden" | "private") {
        status = "waiting".into();
    }
    let raw_date = form.get("date").map(|s| s.as_str()).filter(|s| !s.is_empty()).or_else(|| form.get("created").map(|s| s.as_str())).unwrap_or("");
    let created = parse_local(raw_date, site.timezone())
        .unwrap_or_else(|| existing.as_ref().map(|p| p.created).unwrap_or_else(|| chrono::Utc::now().timestamp()));
    let categories = if many.contains_key("category[]") {
        many.get("category[]").cloned().unwrap_or_default().iter().filter_map(|n| n.parse().ok()).collect()
    } else {
        form.get("categories").map(|s| s.split(',').filter_map(|n| n.trim().parse().ok()).collect()).unwrap_or_else(|| existing.as_ref().map(|p| p.categories.iter().map(|c| c.mid).collect()).unwrap_or_default())
    };
    let tags = if form.contains_key("tags") {
        form.get("tags").map(|s| s.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()).unwrap_or_default()
    } else {
        existing.as_ref().map(|p| p.tags.iter().map(|t| t.name.clone()).collect()).unwrap_or_default()
    };
    let submitted = form.contains_key("do") || form.contains_key("title");
    let allow = |key: &str, old: bool| if submitted { form.contains_key(key) } else { old };
    let saved = state
        .db
        .save_content(&ContentInput {
            cid: existing.as_ref().map(|p| p.cid),
            title: form.get("title").cloned().unwrap_or_else(|| existing.as_ref().map(|p| p.title.clone()).unwrap_or_else(|| "未命名".into())),
            slug: form.get("slug").cloned().unwrap_or_else(|| existing.as_ref().map(|p| p.slug.clone()).unwrap_or_default()),
            text: form.get("text").cloned().unwrap_or_else(|| existing.as_ref().map(|p| p.text.clone()).unwrap_or_default()),
            created,
            kind: kind.into(),
            status,
            password,
            allow_comment: allow("allowComment", existing.as_ref().map(|p| p.allow_comment).unwrap_or(true)),
            allow_ping: allow("allowPing", existing.as_ref().map(|p| p.allow_ping).unwrap_or(true)),
            allow_feed: allow("allowFeed", existing.as_ref().map(|p| p.allow_feed).unwrap_or(true)),
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

fn resolve_status(form: &HashMap<String, String>, _user: &SessionUser, fallback: &str) -> (String, String) {
    let password = form.get("password").cloned().unwrap_or_default();
    if let Some(vis) = form.get("visibility") {
        let mut status = match vis.as_str() {
            "hidden" => "hidden",
            "private" => "private",
            "waiting" => "waiting",
            "password" | "publish" => "publish",
            other => other,
        }
        .to_string();
        let mut password = if vis == "password" { password } else { String::new() };
        if form.get("do").map(|s| s.as_str()) == Some("save") {
            status = "draft".into();
            password.clear();
        }
        return (status, password);
    }
    if let Some(status) = form.get("status").cloned() {
        return (status, password);
    }
    match form.get("do").map(|s| s.as_str()) {
        Some("save") => ("draft".into(), String::new()),
        Some("publish") => ("publish".into(), password),
        _ => (fallback.into(), password),
    }
}

async fn posts_batch(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>, body: String) -> Response {
    content_batch(&state, &headers, "post", "/admin/posts", &q, &body).await
}
async fn pages_batch(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>, body: String) -> Response {
    content_batch(&state, &headers, "page", "/admin/pages", &q, &body).await
}
async fn media_batch(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>, body: String) -> Response {
    content_batch(&state, &headers, "attachment", "/admin/media", &q, &body).await
}

async fn content_batch(
    state: &AppState,
    headers: &HeaderMap,
    kind: &str,
    back: &str,
    q: &HashMap<String, String>,
    body: &str,
) -> Response {
    let need_rank = if kind == "page" { "editor" } else { "contributor" };
    let Ok((_, user, _)) = need(state, headers, need_rank).await else {
        return redirect("/admin/login");
    };
    let ids: Vec<i64> = repeated(body).get("cid[]").cloned().unwrap_or_default().iter().filter_map(|s| s.parse().ok()).collect();
    let action = q.get("do").map(|s| s.as_str()).unwrap_or("");
    let mark = q.get("status").map(|s| s.as_str()).unwrap_or("");
    for cid in ids {
        let Ok(Some(post)) = state.db.content_by_id(cid).await else { continue };
        if post.kind != kind && !(kind == "post" && post.kind == "post") {
            continue;
        }
        if !user.pass("editor") && post.author_id != user.uid {
            continue;
        }
        if action == "delete" {
            let _ = state.db.delete_content(cid).await;
        } else if action == "mark" && user.pass("editor") {
            let _ = state.db.set_content_status(cid, mark).await;
        }
    }
    redirect(back)
}

async fn content_delete(State(state): State<Arc<AppState>>, headers: HeaderMap, Path(cid): Path<i64>) -> Response {
    delete_kind(&state, &headers, cid, "/admin/posts").await
}
async fn page_delete(State(state): State<Arc<AppState>>, headers: HeaderMap, Path(cid): Path<i64>) -> Response {
    delete_kind(&state, &headers, cid, "/admin/pages").await
}
async fn media_delete(State(state): State<Arc<AppState>>, headers: HeaderMap, Path(cid): Path<i64>) -> Response {
    delete_kind(&state, &headers, cid, "/admin/media").await
}

async fn delete_kind(state: &AppState, headers: &HeaderMap, cid: i64, back: &str) -> Response {
    let Ok((_, user, _)) = need(state, headers, "contributor").await else {
        return redirect("/admin/login");
    };
    let Ok(Some(post)) = state.db.content_by_id(cid).await else {
        return plain(StatusCode::NOT_FOUND, "内容不存在");
    };
    if post.kind == "page" && !user.pass("editor") {
        return plain(StatusCode::FORBIDDEN, "独立页需要编辑权限");
    }
    if !user.pass("editor") && post.author_id != user.uid {
        return plain(StatusCode::FORBIDDEN, "不能删除这条内容");
    }
    let _ = state.db.delete_content(cid).await;
    redirect(back)
}

async fn categories(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    meta_page(&state, &headers, "category", "管理分类", "categories", "/admin/categories", "/admin/categories", &q).await
}
async fn tags(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    meta_page(&state, &headers, "tag", "管理标签", "tags", "/admin/tags", "/admin/tags", &q).await
}

async fn meta_page(
    state: &AppState,
    headers: &HeaderMap,
    kind: &str,
    title: &str,
    key: &str,
    base: &str,
    back: &str,
    q: &HashMap<String, String>,
) -> Response {
    let Ok((site, user, _)) = need(state, headers, "editor").await else {
        return redirect("/admin/login");
    };
    let keywords = q.get("keywords").map(|s| s.as_str()).unwrap_or("");
    let rows = state.db.list_metas(kind).await.unwrap_or_default();
    let rows: Vec<_> = rows.into_iter().filter(|m| keywords.is_empty() || title_hit(&m.name, keywords)).collect();
    let mut body_rows = String::new();
    if rows.is_empty() {
        body_rows.push_str("<tr><td colspan=\"5\"><h6 class=\"typecho-list-table-title\">没有任何项目</h6></td></tr>");
    }
    for row in &rows {
        let link = if kind == "category" {
            links::category_path(&site, row)
        } else {
            links::tag_path(&site, row)
        };
        body_rows.push_str(&format!(
            r#"<tr id="mid-{mid}"><td class="kit-hidden-mb"><input type="checkbox" value="{mid}" name="mid[]"></td>
<td><a href="{base}?mid={mid}">{name}</a> <a href="{link}"><i class="i-exlink"></i></a></td>
<td class="kit-hidden-mb">{slug}</td><td>{count}</td>
<td><form method="post" action="{base}/{mid}/delete"><button class="btn btn-s" type="submit">删除</button></form></td></tr>"#,
            mid = row.mid,
            name = html_escape(&row.name),
            link = html_escape(&link),
            slug = html_escape(&row.slug),
            count = row.count,
        ));
    }
    let parent = if kind == "category" {
        "<li><label class=\"typecho-label\">父级</label><input class=\"text-s\" name=\"parent\" value=\"0\"><p class=\"description\">父分类的编号，0 表示无父级。</p></li>"
    } else {
        ""
    };
    let main = format!(
        r#"<div class="row typecho-page-main manage-metas"><div class="col-mb-12" role="main">
<div class="typecho-list-operate clearfix"><form method="get"><div class="operate">
<label><i class="sr-only">全选</i><input type="checkbox" class="typecho-table-select-all"></label>
<div class="btn-group btn-drop"><button class="btn dropdown-toggle btn-s" type="button">选中项 <i class="i-caret-down"></i></button>
<ul class="dropdown-menu"><li><a lang="你确认要删除这些项目吗?" href="{base}/batch?do=delete">删除</a></li></ul></div>
</div><div class="search"><input class="text-s" name="keywords" placeholder="请输入关键字" value="{kw}"><button class="btn btn-s" type="submit">筛选</button></div></form></div>
<form method="post" class="operate-form" action="{base}/batch"><div class="typecho-table-wrap"><table class="typecho-list-table">
<thead><tr><th class="kit-hidden-mb"></th><th>名称</th><th class="kit-hidden-mb">缩略名</th><th>数量</th><th></th></tr></thead>
<tbody>{body_rows}</tbody></table></div></form>
<form method="post" action="{back}"><ul class="typecho-option" id="custom-field">
<li><label class="typecho-label">名称</label><input class="text w-100" name="name" required></li>
<li><label class="typecho-label">缩略名</label><input class="text w-100 mono" name="slug"></li>
<li><label class="typecho-label">描述</label><textarea name="description"></textarea></li>
{parent}
<li class="typecho-option-submit"><button type="submit" class="btn primary">保存</button></li>
</ul></form>
</div></div>"#,
        kw = html_escape(keywords)
    );
    layout(&site, &user, key, title, None, "container", &main)
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
            mid: form.get("mid").and_then(|s| s.parse().ok()),
            name: form.get("name").cloned().unwrap_or_default(),
            slug: form.get("slug").cloned().unwrap_or_default(),
            kind: kind.into(),
            description: form.get("description").cloned().unwrap_or_default(),
            parent: form.get("parent").and_then(|s| s.parse().ok()).unwrap_or(0),
        })
        .await;
    redirect(back)
}

async fn categories_batch(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>, body: String) -> Response {
    meta_batch(&state, &headers, "/admin/categories", &q, &body).await
}
async fn tags_batch(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>, body: String) -> Response {
    meta_batch(&state, &headers, "/admin/tags", &q, &body).await
}

async fn meta_batch(state: &AppState, headers: &HeaderMap, back: &str, q: &HashMap<String, String>, body: &str) -> Response {
    let Ok((_, _, _)) = need(state, headers, "editor").await else {
        return redirect("/admin/login");
    };
    if q.get("do").map(|s| s.as_str()) == Some("delete") {
        for mid in repeated(body).get("mid[]").cloned().unwrap_or_default() {
            if let Ok(mid) = mid.parse::<i64>() {
                let _ = state.db.delete_meta(mid).await;
            }
        }
    }
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

async fn comments(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let Ok((site, user, _)) = need(&state, &headers, "contributor").await else {
        return redirect("/admin/login");
    };
    let rows = state.db.list_comments(500).await.unwrap_or_default();
    let posts = state.db.list_contents("post", 500).await.unwrap_or_default();
    let pages = state.db.list_contents("page", 200).await.unwrap_or_default();
    let mut titles: HashMap<i64, String> = HashMap::new();
    let mut owners: HashMap<i64, i64> = HashMap::new();
    for p in posts.iter().chain(pages.iter()) {
        titles.insert(p.cid, p.title.clone());
        owners.insert(p.cid, p.author_id);
    }
    let status = q.get("status").map(|s| s.as_str()).unwrap_or("approved");
    let keywords = q.get("keywords").map(|s| s.as_str()).unwrap_or("");
    let cid = q.get("cid").and_then(|s| s.parse::<i64>().ok());
    let all = q.get("__typecho_all_comments").map(|s| s == "on").unwrap_or(user.pass("editor"));
    let filtered: Vec<_> = rows
        .iter()
        .filter(|c| c.status == status || (status == "approved" && q.get("status").is_none() && c.status == "approved"))
        .filter(|c| cid.is_none_or(|id| c.cid == id))
        .filter(|c| {
            if user.pass("editor") && all {
                true
            } else {
                owners.get(&c.cid).copied().unwrap_or(c.owner_id) == user.uid
            }
        })
        .filter(|c| keywords.is_empty() || title_hit(&c.text, keywords) || title_hit(&c.author, keywords))
        .collect();
    let page = q.get("page").and_then(|s| s.parse::<usize>().ok()).unwrap_or(1).max(1);
    let pages_n = filtered.len().div_ceil(PAGE).max(1);
    let page = page.min(pages_n);
    let slice = &filtered[(page - 1) * PAGE..filtered.len().min(page * PAGE)];
    let waiting = rows.iter().filter(|c| c.status == "waiting").count();
    let spam = rows.iter().filter(|c| c.status == "spam").count();
    let mut tabs = String::new();
    for (name, label, n) in [("approved", "已通过", 0usize), ("waiting", "待审核", waiting), ("spam", "垃圾", spam)] {
        let current = if status == name { " class=\"current\"" } else { "" };
        let balloon = if n > 0 { format!(" <span class=\"balloon\">{n}</span>") } else { String::new() };
        tabs.push_str(&format!("<li{current}><a href=\"/admin/comments?status={name}\">{label}{balloon}</a></li>"));
    }
    let mut body_rows = String::new();
    if slice.is_empty() {
        body_rows.push_str("<tr><td colspan=\"4\"><h6 class=\"typecho-list-table-title\">没有评论</h6></td></tr>");
    }
    for row in slice {
        let title = titles.get(&row.cid).map(|s| s.as_str()).unwrap_or("文章");
        let weak = |name: &str, label: &str| {
            if row.status == name {
                format!("<span class=\"weak\">{label}</span>")
            } else {
                format!("<a href=\"/admin/comments/act?do={name}&coid={}\">{label}</a>", row.coid)
            }
        };
        body_rows.push_str(&format!(
            r#"<tr id="comment-{coid}"><td valign="top" class="kit-hidden-mb"><input type="checkbox" value="{coid}" name="coid[]"></td>
<td valign="top" class="kit-hidden-mb"><div class="comment-avatar">{avatar}</div></td>
<td valign="top" class="comment-head"><div class="comment-meta"><strong class="comment-author">{author}</strong><br><span>{mail}</span></div></td>
<td valign="top" class="comment-body"><div class="comment-date">{date} 于 <a href="/admin/comments?cid={cid}">{title}</a></div>
<div class="comment-content"><p>{text}</p></div>
<div class="comment-action hidden-by-mouse">{pass} {wait} {spam} <a lang="你确认要删除这条评论吗?" href="/admin/comments/act?do=delete&coid={coid}">删除</a></div>
</td></tr>"#,
            coid = row.coid,
            avatar = gravatar(&state.cfg.gravatar_prefix, &row.mail),
            author = html_escape(&row.author),
            mail = html_escape(&row.mail),
            date = html_escape(&date_word(row.created, site.timezone())),
            cid = row.cid,
            title = html_escape(title),
            text = html_escape(&crate::text::truncate_chars(&row.text, 280)),
            pass = weak("approved", "通过"),
            wait = weak("waiting", "待审核"),
            spam = weak("spam", "垃圾"),
        ));
    }
    let pager = pager_html(page, pages_n, "/admin/comments", &q);
    let main = format!(
        r#"<div class="row typecho-page-main" role="main"><div class="col-mb-12 typecho-list">
<div class="clearfix"><ul class="typecho-option-tabs">{tabs}</ul></div>
<div class="typecho-list-operate clearfix"><form method="get"><div class="operate">
<label><i class="sr-only">全选</i><input type="checkbox" class="typecho-table-select-all"></label>
<div class="btn-group btn-drop"><button class="btn dropdown-toggle btn-s" type="button">选中项 <i class="i-caret-down"></i></button>
<ul class="dropdown-menu">
<li><a href="/admin/comments/batch?do=approved">通过</a></li>
<li><a href="/admin/comments/batch?do=waiting">待审核</a></li>
<li><a href="/admin/comments/batch?do=spam">标记垃圾</a></li>
<li><a lang="你确认要删除这些评论吗?" href="/admin/comments/batch?do=delete">删除</a></li>
</ul></div></div>
<div class="search" role="search"><input type="text" class="text-s" placeholder="请输入关键字" name="keywords" value="{kw}">
<input type="hidden" name="status" value="{status}"><button type="submit" class="btn btn-s">筛选</button></div>
</form></div>
<form method="post" class="operate-form" action="/admin/comments/batch"><div class="typecho-table-wrap"><table class="typecho-list-table">
<colgroup><col width="3%" class="kit-hidden-mb"><col width="6%" class="kit-hidden-mb"><col width="20%"><col width="71%"></colgroup>
<thead><tr><th class="kit-hidden-mb"></th><th>作者</th><th class="kit-hidden-mb"></th><th>内容</th></tr></thead>
<tbody>{body_rows}</tbody></table></div></form>
<div class="typecho-list-operate clearfix">{pager}</div>
</div></div>"#,
        kw = html_escape(keywords)
    );
    layout(&site, &user, "comments", "管理评论", None, "container", &main)
}

async fn comments_batch(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>, body: String) -> Response {
    let Ok((_, _, _)) = need(&state, &headers, "contributor").await else {
        return redirect("/admin/login");
    };
    let action = q.get("do").map(|s| s.as_str()).unwrap_or("");
    if action == "delete-spam" {
        if let Ok(rows) = state.db.list_comments(1000).await {
            for row in rows.into_iter().filter(|c| c.status == "spam") {
                let _ = state.db.delete_comment(row.coid).await;
            }
        }
    } else {
        for coid in repeated(&body).get("coid[]").cloned().unwrap_or_default() {
            if let Ok(coid) = coid.parse::<i64>() {
                apply_comment(&state, coid, action).await;
            }
        }
    }
    redirect("/admin/comments")
}

async fn comment_get_act(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let Ok((_, _, _)) = need(&state, &headers, "contributor").await else {
        return redirect("/admin/login");
    };
    if let Some(coid) = q.get("coid").and_then(|s| s.parse().ok()) {
        apply_comment(&state, coid, q.get("do").map(|s| s.as_str()).unwrap_or("")).await;
    }
    let status = q.get("status").map(|s| s.as_str()).unwrap_or("");
    if status.is_empty() {
        redirect("/admin/comments")
    } else {
        redirect(&format!("/admin/comments?status={status}"))
    }
}

async fn comment_act(State(state): State<Arc<AppState>>, headers: HeaderMap, Path(coid): Path<i64>, body: String) -> Response {
    let Ok((_, _, _)) = need(&state, &headers, "contributor").await else {
        return redirect("/admin/login");
    };
    let form = pairs(&body);
    let act = form.get("act").map(|s| s.as_str()).or_else(|| form.get("do").map(|s| s.as_str())).unwrap_or("");
    apply_comment(&state, coid, act).await;
    redirect("/admin/comments")
}

async fn apply_comment(state: &AppState, coid: i64, act: &str) {
    if act == "delete" {
        let _ = state.db.delete_comment(coid).await;
    } else if act == "approved" || act == "spam" || act == "waiting" {
        let _ = state.db.set_comment_status(coid, act).await;
    }
}

async fn media(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let Ok((site, user, _)) = need(&state, &headers, "contributor").await else {
        return redirect("/admin/login");
    };
    let rows = state.db.list_contents("attachment", 300).await.unwrap_or_default();
    let keywords = q.get("keywords").map(|s| s.as_str()).unwrap_or("");
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|p| user.pass("editor") || p.author_id == user.uid)
        .filter(|p| keywords.is_empty() || title_hit(&p.title, keywords))
        .collect();
    let mut body_rows = String::new();
    if rows.is_empty() {
        body_rows.push_str("<tr><td colspan=\"4\"><h6 class=\"typecho-list-table-title\">没有任何文件</h6></td></tr>");
    }
    for row in &rows {
        body_rows.push_str(&format!(
            r#"<tr><td class="kit-hidden-mb"><input type="checkbox" name="cid[]" value="{cid}"></td>
<td><a href="/attachment/{cid}/">{title}</a> <a href="/attachment/{cid}/"><i class="i-exlink"></i></a></td>
<td>{date}</td><td>{author}</td></tr>"#,
            cid = row.cid,
            title = html_escape(&row.title),
            date = html_escape(&date_word(row.created, site.timezone())),
            author = html_escape(&row.display_author()),
        ));
    }
    let main = format!(
        r#"<div class="row typecho-page-main"><div class="col-mb-12 typecho-list">
<div class="typecho-list-operate clearfix"><form method="get"><div class="operate">
<label><i class="sr-only">全选</i><input type="checkbox" class="typecho-table-select-all"></label>
<div class="btn-group btn-drop"><button class="btn dropdown-toggle btn-s" type="button">选中项 <i class="i-caret-down"></i></button>
<ul class="dropdown-menu"><li><a lang="你确认要删除这些文件吗?" href="/admin/media/batch?do=delete">删除</a></li></ul></div>
</div><div class="search"><input class="text-s" name="keywords" placeholder="请输入关键字" value="{kw}"><button class="btn btn-s">筛选</button></div></form></div>
<form method="post" class="operate-form"><div class="typecho-table-wrap"><table class="typecho-list-table">
<thead><tr><th></th><th>文件名</th><th>上传日期</th><th>作者</th></tr></thead><tbody>{body_rows}</tbody></table></div></form>
<form method="post" action="/admin/media" enctype="multipart/form-data"><ul class="typecho-option">
<li><label class="typecho-label">上传文件</label><input type="file" name="file" required></li>
<li><label class="typecho-label">所属文章</label><input class="text-s" name="parent" placeholder="文章编号，可留空"></li>
<li class="typecho-option-submit"><button class="btn primary" type="submit">上传</button></li>
</ul></form>
</div></div>"#,
        kw = html_escape(keywords)
    );
    layout(&site, &user, "media", "管理文件", None, "container", &main)
}

async fn media_upload(State(state): State<Arc<AppState>>, headers: HeaderMap, mut multipart: Multipart) -> Response {
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
    let folder = stamp.map(|d| format!("{}/{}", d.format("%Y"), d.format("%m"))).unwrap_or_else(|| "0000/00".into());
    let file_name = format!("{}-{safe}", &auth::md5_hex(&bytes)[..12]);
    let root = state.cfg.uploads_dir.clone().unwrap_or_else(|| std::path::PathBuf::from("usr/uploads"));
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

async fn users(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let Ok((site, user, _)) = need(&state, &headers, "administrator").await else {
        return redirect("/admin/login");
    };
    let keywords = q.get("keywords").map(|s| s.as_str()).unwrap_or("");
    let rows = state.db.list_users().await.unwrap_or_default();
    let mut body_rows = String::new();
    for row in rows.iter().filter(|r| keywords.is_empty() || title_hit(&r.name, keywords) || title_hit(&r.screen, keywords)) {
        body_rows.push_str(&format!(
            r#"<tr id="user-{uid}"><td class="kit-hidden-mb"><input type="checkbox" name="uid[]" value="{uid}"></td>
<td><a href="/admin/users?uid={uid}">{name}</a></td>
<td class="kit-hidden-mb">{screen}</td>
<td class="kit-hidden-mb">{mail}</td>
<td>{group}</td></tr>"#,
            uid = row.uid,
            name = html_escape(&row.name),
            screen = html_escape(&row.label()),
            mail = html_escape(&row.mail),
            group = group_label(&row.group),
        ));
    }
    let main = format!(
        r#"<div class="row typecho-page-main"><div class="col-mb-12 typecho-list">
<div class="typecho-list-operate clearfix"><form method="get"><div class="operate">
<label><i class="sr-only">全选</i><input type="checkbox" class="typecho-table-select-all"></label>
<div class="btn-group btn-drop"><button class="btn dropdown-toggle btn-s" type="button">选中项 <i class="i-caret-down"></i></button>
<ul class="dropdown-menu"><li><a lang="你确认要删除这些用户吗?" href="/admin/users/batch?do=delete">删除</a></li></ul></div>
</div><div class="search"><input class="text-s" name="keywords" placeholder="请输入关键字" value="{kw}"><button class="btn btn-s">筛选</button></div></form></div>
<form method="post" class="operate-form"><div class="typecho-table-wrap"><table class="typecho-list-table">
<thead><tr><th></th><th>用户名</th><th class="kit-hidden-mb">昵称</th><th class="kit-hidden-mb">电子邮件</th><th>用户组</th></tr></thead>
<tbody>{body_rows}</tbody></table></div></form>
<form method="post" action="/admin/users"><ul class="typecho-option">
<li><label class="typecho-label">用户名</label><input class="text" name="name" required></li>
<li><label class="typecho-label">昵称</label><input class="text" name="screen"></li>
<li><label class="typecho-label">电子邮件</label><input class="text" name="mail" type="email"></li>
<li><label class="typecho-label">个人主页</label><input class="text" name="url"></li>
<li><label class="typecho-label">密码</label><input class="text" name="password" type="password"><p class="description">更新已有用户时留空则不修改。编号留空表示新建。</p></li>
<li><label class="typecho-label">编号</label><input class="text-s" name="uid"></li>
<li><label class="typecho-label">用户组</label><select name="group"><option value="subscriber">关注者</option><option value="contributor">贡献者</option><option value="editor">编辑</option><option value="administrator">管理员</option></select></li>
<li class="typecho-option-submit"><button class="btn primary" type="submit">保存</button></li>
</ul></form>
</div></div>"#,
        kw = html_escape(keywords)
    );
    layout(&site, &user, "users", "管理用户", None, "container", &main)
}

fn group_label(group: &str) -> &'static str {
    match group {
        "administrator" => "管理员",
        "editor" => "编辑",
        "contributor" => "贡献者",
        "subscriber" => "关注者",
        _ => "访问者",
    }
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
            screen: form.get("screen").cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| form.get("name").cloned().unwrap_or_default()),
            group: form.get("group").cloned().unwrap_or_else(|| "subscriber".into()),
            created: now,
        })
        .await;
    redirect("/admin/users")
}

async fn users_batch(State(state): State<Arc<AppState>>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>, body: String) -> Response {
    let Ok((_, user, _)) = need(&state, &headers, "administrator").await else {
        return redirect("/admin/login");
    };
    if q.get("do").map(|s| s.as_str()) == Some("delete") {
        let rows = state.db.list_users().await.unwrap_or_default();
        let admins = rows.iter().filter(|r| r.group == "administrator").count();
        for uid in repeated(&body).get("uid[]").cloned().unwrap_or_default() {
            let Ok(uid) = uid.parse::<i64>() else { continue };
            if uid == user.uid {
                continue;
            }
            let target = rows.iter().find(|r| r.uid == uid);
            if target.is_some_and(|r| r.group == "administrator") && admins <= 1 {
                continue;
            }
            let _ = state.db.delete_user(uid).await;
        }
    }
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

async fn settings_general(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let Ok((site, user, _)) = need(&state, &headers, "administrator").await else {
        return redirect("/admin/login");
    };
    let tz = site.int("timezone", 28800);
    let mut zones = String::new();
    for (value, label) in TIMEZONES {
        let sel = if tz == *value { " selected" } else { "" };
        zones.push_str(&format!("<option value=\"{value}\"{sel}>{label}</option>"));
    }
    let main = format!(
        r#"<div class="row typecho-page-main" role="form"><div class="col-mb-12 col-tb-8 col-tb-offset-2">
<form method="post" action="/admin/settings/general"><ul class="typecho-option">
<li><label class="typecho-label" for="title">站点名称</label><input id="title" name="title" type="text" class="w-100 text" value="{title}"><p class="description">站点的名称将显示在网页的标题处.</p></li>
<li><label class="typecho-label" for="siteUrl">站点地址</label><input id="siteUrl" name="siteUrl" type="text" class="w-100 mono" value="{site}"><p class="description">站点地址主要用于生成内容的永久链接.</p></li>
<li><label class="typecho-label" for="description">站点描述</label><input id="description" name="description" type="text" class="w-100 text" value="{desc}"><p class="description">站点描述将显示在网页代码的头部.</p></li>
<li><label class="typecho-label" for="keywords">关键词</label><input id="keywords" name="keywords" type="text" class="w-100 text" value="{kw}"><p class="description">请以半角逗号 "," 分割多个关键字.</p></li>
<li><label class="typecho-label">是否允许注册</label>{reg}<p class="description">允许访问者注册到你的网站, 默认的注册用户不享有任何写入权限.</p></li>
<li><label class="typecho-label">XMLRPC 接口</label>{xml}</li>
<li><label class="typecho-label" for="timezone">时区</label><select id="timezone" name="timezone">{zones}</select></li>
<li class="typecho-option-submit"><button type="submit" class="btn primary">保存设置</button></li>
</ul></form></div></div>"#,
        title = html_escape(site.get("title")),
        site = html_escape(site.get("siteUrl")),
        desc = html_escape(site.get("description")),
        kw = html_escape(site.get("keywords")),
        reg = radios("allowRegister", &[("0", "不允许"), ("1", "允许")], site.get_or("allowRegister", "0")),
        xml = radios("allowXmlRpc", &[("0", "关闭"), ("1", "仅关闭 Pingback 接口"), ("2", "打开")], site.get_or("allowXmlRpc", "2")),
    );
    layout(&site, &user, "general", "基本设置", None, "container", &main)
}

async fn settings_discussion(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let Ok((site, user, _)) = need(&state, &headers, "administrator").await else {
        return redirect("/admin/login");
    };
    let days = site.int("commentsPostTimeout", 0) / 86400;
    let minutes = site.int("commentsPostInterval", 60) as f64 / 60.0;
    let show = checkbox_group(
        "commentsShow",
        &[
            ("commentsShowCommentOnly", "仅显示评论, 不显示 Pingback 和 Trackback"),
            ("commentsMarkdown", "在评论中使用 Markdown 语法"),
            ("commentsShowUrl", "评论者名称显示时自动加上其个人主页链接"),
            ("commentsUrlNofollow", "对评论者个人主页链接使用 nofollow 属性"),
            ("commentsAvatar", "启用 Gravatar 头像服务"),
            ("commentsPageBreak", "启用分页"),
            ("commentsThreaded", "启用评论回复"),
        ],
        &site,
    );
    let post = checkbox_group(
        "commentsPost",
        &[
            ("commentsRequireModeration", "所有评论必须经过审核"),
            ("commentsWhitelist", "评论者之前须有评论通过了审核"),
            ("commentsRequireMail", "必须填写邮箱"),
            ("commentsRequireURL", "必须填写网址"),
            ("commentsCheckReferer", "检查评论来源页 URL 是否与文章链接一致"),
            ("commentsAntiSpam", "开启反垃圾保护"),
            ("commentsAutoClose", "在文章发布若干天以后自动关闭评论"),
            ("commentsPostIntervalEnable", "同一 IP 发布评论的时间间隔限制"),
        ],
        &site,
    );
    let main = format!(
        r#"<div class="row typecho-page-main" role="form"><div class="col-mb-12 col-tb-8 col-tb-offset-2">
<form method="post" action="/admin/settings/discussion"><ul class="typecho-option">
<li><label class="typecho-label">评论日期格式</label><input class="w-40 mono" name="commentDateFormat" value="{fmt}"><p class="description">具体写法请参考 PHP 日期格式写法.</p></li>
<li><label class="typecho-label">评论列表数目</label><input class="w-20" name="commentsListSize" value="{list}"><p class="description">此数目用于指定显示在侧边栏中的评论列表数目.</p></li>
<li><label class="typecho-label">评论显示</label>{show}
<p class="description">每页评论数 <input class="text num text-s" name="commentsPageSize" value="{page}"> ，默认显示
<select name="commentsPageDisplay"><option value="first"{first}>第一页</option><option value="last"{last}>最后一页</option></select>
，回复层数 <input class="text num text-s" name="commentsMaxNestingLevels" value="{nest}">
，排序 <select name="commentsOrder"><option value="DESC"{desc}>较新的</option><option value="ASC"{asc}>较旧的</option></select></p></li>
<li><label class="typecho-label">评论提交</label>{post}
<p class="description">自动关闭天数 <input class="text num text-s" name="commentsPostTimeout" value="{days}"> ，间隔分钟 <input class="text num text-s" name="commentsPostInterval" value="{minutes}"></p></li>
<li><label class="typecho-label">允许使用的HTML标签和属性</label><textarea class="mono" name="commentsHTMLTagAllowed">{html}</textarea></li>
<li class="typecho-option-submit"><button type="submit" class="btn primary">保存设置</button></li>
</ul></form></div></div>"#,
        fmt = html_escape(site.get("commentDateFormat")),
        list = html_escape(site.get_or("commentsListSize", "10")),
        page = site.int("commentsPageSize", 20),
        nest = html_escape(site.get_or("commentsMaxNestingLevels", "5")),
        first = if site.get("commentsPageDisplay") != "last" { " selected" } else { "" },
        last = if site.get("commentsPageDisplay") == "last" { " selected" } else { "" },
        desc = if !site.get("commentsOrder").eq_ignore_ascii_case("ASC") { " selected" } else { "" },
        asc = if site.get("commentsOrder").eq_ignore_ascii_case("ASC") { " selected" } else { "" },
        days = days,
        minutes = minutes,
        html = html_escape(site.get("commentsHTMLTagAllowed")),
    );
    layout(&site, &user, "discussion", "评论设置", None, "container", &main)
}

async fn settings_reading(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let Ok((site, user, _)) = need(&state, &headers, "administrator").await else {
        return redirect("/admin/login");
    };
    let front = site.get("frontPage");
    let recent = if !front.starts_with("page:") { " checked" } else { "" };
    let page_on = if front.starts_with("page:") { " checked" } else { "" };
    let selected_page = front.strip_prefix("page:").unwrap_or("");
    let mut pages = String::new();
    for p in &site.pages {
        let sel = if p.cid.to_string() == selected_page { " selected" } else { "" };
        pages.push_str(&format!("<option value=\"{}\"{sel}>{}</option>", p.cid, html_escape(&p.title)));
    }
    let archive = if site.flag("frontArchive", false) { " checked" } else { "" };
    let main = format!(
        r#"<div class="row typecho-page-main" role="form"><div class="col-mb-12 col-tb-8 col-tb-offset-2">
<form method="post" action="/admin/settings/reading"><ul class="typecho-option">
<li><label class="typecho-label">文章日期格式</label><input class="w-40 mono" name="postDateFormat" value="{fmt}"><p class="description">此格式用于指定显示在文章归档中的日期默认显示格式.</p></li>
<li><label class="typecho-label">站点首页</label>
<span class="multiline"><input type="radio" name="frontPage" value="recent"{recent}> 显示最新发布的文章</span>
<span class="multiline"><input type="radio" name="frontPage" value="page"{page_on}> 使用 <select name="frontPagePage">{pages}</select> 页面作为首页</span>
<span class="multiline"><input type="checkbox" name="frontArchive" value="1"{archive}> 同时将文章列表页路径更改为 /blog/</span>
</li>
<li><label class="typecho-label">文章列表数目</label><input class="w-20" name="postsListSize" value="{side}"><p class="description">此数目用于指定显示在侧边栏中的文章列表数目.</p></li>
<li><label class="typecho-label">每页文章数目</label><input class="w-20" name="pageSize" value="{page}"><p class="description">此数目用于指定文章归档输出时每页显示的文章数目.</p></li>
<li><label class="typecho-label">聚合全文输出</label>{feed}<p class="description">如果你不希望在聚合中输出文章全文, 请使用仅输出摘要选项. 摘要的文字取决于你在文章中使用分隔符的位置.</p></li>
<li class="typecho-option-submit"><button type="submit" class="btn primary">保存设置</button></li>
</ul></form></div></div>"#,
        fmt = html_escape(site.get_or("postDateFormat", "Y-m-d")),
        side = html_escape(site.get_or("postsListSize", "10")),
        page = site.page_size(),
        feed = radios("feedFullText", &[("0", "仅输出摘要"), ("1", "全文输出")], site.get_or("feedFullText", "0")),
    );
    layout(&site, &user, "reading", "阅读设置", None, "container", &main)
}

async fn settings_permalink(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let Ok((site, user, _)) = need(&state, &headers, "administrator").await else {
        return redirect("/admin/login");
    };
    let post = route_source(&site, "post");
    let page = decode_rule(&route_source(&site, "page"));
    let category = decode_rule(&route_source(&site, "category"));
    let patterns = [
        ("/archives/[cid:digital]/", "默认风格", "/archives/{cid}/"),
        ("/archives/[slug].html", "wordpress风格", "/archives/{slug}.html"),
        ("/[year:digital:4]/[month:digital:2]/[day:digital:2]/[slug].html", "按日期归档", "/{year}/{month}/{day}/{slug}.html"),
        ("/[category]/[slug].html", "按分类归档", "/{category}/{slug}.html"),
    ];
    let custom = !patterns.iter().any(|(p, _, _)| *p == post);
    let mut radios_html = String::new();
    for (value, label, code) in patterns {
        let sel = if post == value { " checked" } else { "" };
        radios_html.push_str(&format!(
            "<span class=\"multiline\"><input type=\"radio\" name=\"postPattern\" value=\"{value}\"{sel}> {label} <code>{code}</code></span>"
        ));
    }
    let custom_checked = if custom { " checked" } else { "" };
    let custom_value = if custom { decode_rule(&post) } else { String::new() };
    radios_html.push_str(&format!(
        "<span class=\"multiline\"><input type=\"radio\" name=\"postPattern\" value=\"custom\"{custom_checked}> 个性化定义 <input type=\"text\" class=\"w-50 text-s mono\" name=\"customPattern\" value=\"{}\"></span>",
        html_escape(&custom_value)
    ));
    let rewrite = radios("rewrite", &[("0", "不启用"), ("1", "启用")], site.get_or("rewrite", "0"));
    let main = format!(
        r#"<div class="row typecho-page-main" role="form"><div class="col-mb-12 col-tb-8 col-tb-offset-2">
<form method="post" action="/admin/settings/permalink"><ul class="typecho-option">
<li><label class="typecho-label">是否使用地址重写功能</label>{rewrite}<p class="description">打开此功能可以让你的链接看上去完全是静态地址.</p></li>
<li><label class="typecho-label">自定义文章路径</label>{radios_html}<p class="description">可用参数: <code>{{cid}}</code> <code>{{slug}}</code> <code>{{category}}</code> <code>{{year}}</code> <code>{{month}}</code> <code>{{day}}</code></p></li>
<li><label class="typecho-label">独立页面路径</label><input class="mono w-60" name="pagePattern" value="{page}"><p class="description">可用参数: <code>{{cid}}</code> <code>{{slug}}</code></p></li>
<li><label class="typecho-label">分类路径</label><input class="mono w-60" name="categoryPattern" value="{category}"><p class="description">可用参数: <code>{{mid}}</code> <code>{{slug}}</code></p></li>
<li class="typecho-option-submit"><button type="submit" class="btn primary">保存设置</button></li>
</ul></form></div></div>"#,
        page = html_escape(&page),
        category = html_escape(&category),
    );
    layout(&site, &user, "permalink", "永久链接设置", None, "container", &main)
}

async fn settings_save_general(State(state): State<Arc<AppState>>, headers: HeaderMap, body: String) -> Response {
    let Ok((_, _, _)) = need(&state, &headers, "administrator").await else {
        return redirect("/admin/login");
    };
    let form = pairs(&body);
    for key in ["title", "siteUrl", "description", "keywords", "allowRegister", "allowXmlRpc", "timezone"] {
        if let Some(value) = form.get(key) {
            let _ = state.db.set_option(key, value).await;
        }
    }
    redirect("/admin/settings/general")
}

async fn settings_save_discussion(State(state): State<Arc<AppState>>, headers: HeaderMap, body: String) -> Response {
    let Ok((_, _, _)) = need(&state, &headers, "administrator").await else {
        return redirect("/admin/login");
    };
    let form = pairs(&body);
    let many = repeated(&body);
    for key in [
        "commentDateFormat",
        "commentsListSize",
        "commentsPageSize",
        "commentsPageDisplay",
        "commentsMaxNestingLevels",
        "commentsOrder",
        "commentsHTMLTagAllowed",
    ] {
        if let Some(value) = form.get(key) {
            let _ = state.db.set_option(key, value).await;
        }
    }
    if let Some(days) = form.get("commentsPostTimeout").and_then(|s| s.parse::<i64>().ok()) {
        let _ = state.db.set_option("commentsPostTimeout", &(days.saturating_mul(86400)).to_string()).await;
    }
    if let Some(minutes) = form.get("commentsPostInterval").and_then(|s| s.parse::<f64>().ok()) {
        let secs = (minutes * 60.0).round() as i64;
        let _ = state.db.set_option("commentsPostInterval", &secs.to_string()).await;
    }
    let flags = [
        "commentsShowCommentOnly",
        "commentsMarkdown",
        "commentsShowUrl",
        "commentsUrlNofollow",
        "commentsAvatar",
        "commentsPageBreak",
        "commentsThreaded",
        "commentsRequireModeration",
        "commentsWhitelist",
        "commentsRequireMail",
        "commentsRequireURL",
        "commentsCheckReferer",
        "commentsAntiSpam",
        "commentsAutoClose",
        "commentsPostIntervalEnable",
    ];
    let chosen = many.get("commentsShow").cloned().unwrap_or_default();
    let chosen_post = many.get("commentsPost").cloned().unwrap_or_default();
    for key in flags {
        let on = chosen.iter().any(|v| v == key) || chosen_post.iter().any(|v| v == key);
        let _ = state.db.set_option(key, if on { "1" } else { "0" }).await;
    }
    redirect("/admin/settings/discussion")
}

async fn settings_save_reading(State(state): State<Arc<AppState>>, headers: HeaderMap, body: String) -> Response {
    let Ok((_, _, _)) = need(&state, &headers, "administrator").await else {
        return redirect("/admin/login");
    };
    let form = pairs(&body);
    for key in ["postDateFormat", "postsListSize", "pageSize", "feedFullText"] {
        if let Some(value) = form.get(key) {
            let _ = state.db.set_option(key, value).await;
        }
    }
    let front = if form.get("frontPage").map(|s| s.as_str()) == Some("page") {
        format!("page:{}", form.get("frontPagePage").map(|s| s.as_str()).unwrap_or("0"))
    } else {
        "recent".into()
    };
    let _ = state.db.set_option("frontPage", &front).await;
    let _ = state.db.set_option("frontArchive", if form.contains_key("frontArchive") { "1" } else { "0" }).await;
    redirect("/admin/settings/reading")
}

async fn settings_save_permalink(State(state): State<Arc<AppState>>, headers: HeaderMap, body: String) -> Response {
    let Ok((site, _, _)) = need(&state, &headers, "administrator").await else {
        return redirect("/admin/login");
    };
    let form = pairs(&body);
    if let Some(rewrite) = form.get("rewrite") {
        let _ = state.db.set_option("rewrite", rewrite).await;
    }
    let post = if form.get("postPattern").map(|s| s.as_str()) == Some("custom") {
        encode_rule(form.get("customPattern").map(|s| s.as_str()).unwrap_or(""))
    } else {
        form.get("postPattern").cloned().unwrap_or_else(|| route_source(&site, "post"))
    };
    let page = encode_rule(form.get("pagePattern").map(|s| s.as_str()).unwrap_or(""));
    let category = encode_rule(form.get("categoryPattern").map(|s| s.as_str()).unwrap_or(""));
    if let Some(blob) = patch_routes(&site, &post, &page, &category) {
        let _ = state.db.set_option("routingTable", &blob).await;
    }
    redirect("/admin/settings/permalink")
}

fn patch_routes(site: &Site, post: &str, page: &str, category: &str) -> Option<String> {
    let raw = site.get("routingTable");
    let mut value = if raw.is_empty() {
        let entries = site
            .routes
            .routes
            .iter()
            .map(|r| {
                (
                    PhpValue::String(r.name.clone()),
                    PhpValue::Array(vec![(PhpValue::String("url".into()), PhpValue::String(r.source.clone()))]),
                )
            })
            .collect();
        PhpValue::Array(entries)
    } else {
        phpval::parse_php(raw).ok()?
    };
    let category_page = format!("{}/[page:digital]/", category.trim_end_matches('/'));
    for (name, url) in [("post", post), ("page", page), ("category", category), ("category_page", category_page.as_str())] {
        if url.is_empty() || url == "/" {
            continue;
        }
        set_route_url(&mut value, name, url);
    }
    Some(phpval::serialize(&value))
}

fn set_route_url(value: &mut PhpValue, name: &str, url: &str) {
    let PhpValue::Array(items) = value else { return };
    for (key, val) in items.iter_mut() {
        if key.as_str() != Some(name) {
            continue;
        }
        if let PhpValue::Array(fields) = val {
            if let Some((_, slot)) = fields.iter_mut().find(|(k, _)| k.as_str() == Some("url")) {
                *slot = PhpValue::String(url.into());
            } else {
                fields.push((PhpValue::String("url".into()), PhpValue::String(url.into())));
            }
            return;
        }
    }
    items.push((
        PhpValue::String(name.into()),
        PhpValue::Array(vec![(PhpValue::String("url".into()), PhpValue::String(url.into()))]),
    ));
}

fn route_source(site: &Site, name: &str) -> String {
    site.routes.routes.iter().find(|r| r.name == name).map(|r| r.source.clone()).unwrap_or_default()
}

fn encode_rule(rule: &str) -> String {
    let rule = rule.trim();
    if rule.is_empty() {
        return String::new();
    }
    let mut s = rule.to_string();
    for (from, to) in [
        ("{cid}", "[cid:digital]"),
        ("{year}", "[year:digital:4]"),
        ("{month}", "[month:digital:2]"),
        ("{day}", "[day:digital:2]"),
        ("{mid}", "[mid:digital]"),
        ("{directory}", "[directory:split:0]"),
        ("{slug}", "[slug]"),
        ("{category}", "[category]"),
    ] {
        s = s.replace(from, to);
    }
    if !s.starts_with('/') {
        s.insert(0, '/');
    }
    s
}

fn decode_rule(rule: &str) -> String {
    let mut out = String::new();
    let bytes = rule.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some(end) = rule[i + 1..].find(']') {
                let inner = &rule[i + 1..i + 1 + end];
                let name = inner.split(':').next().unwrap_or(inner);
                out.push('{');
                out.push_str(name);
                out.push('}');
                i += end + 2;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn radios(name: &str, options: &[(&str, &str)], current: &str) -> String {
    let mut html = String::new();
    for (value, label) in options {
        let sel = if *value == current { " checked" } else { "" };
        html.push_str(&format!(
            "<span class=\"multiline\"><input type=\"radio\" name=\"{name}\" value=\"{value}\"{sel}> {label}</span>"
        ));
    }
    html
}

fn checkbox_group(name: &str, options: &[(&str, &str)], site: &Site) -> String {
    let mut html = String::new();
    for (key, label) in options {
        let sel = if site.flag(key, false) { " checked" } else { "" };
        html.push_str(&format!(
            "<span class=\"multiline\"><input type=\"checkbox\" name=\"{name}\" value=\"{key}\"{sel}> {label}</span>"
        ));
    }
    html
}

fn layout(site: &Site, user: &SessionUser, key: &str, title: &str, add: Option<&str>, container: &str, main: &str) -> Response {
    let add_html = add
        .map(|href| format!("<a href=\"{href}\">新增</a>"))
        .unwrap_or_default();
    let html = format!(
        r#"<!DOCTYPE HTML>
<html><head>
<meta charset="UTF-8">
<meta name="renderer" content="webkit">
<meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no">
<title>{title} - {site_title} - Powered by Typecho</title>
<meta name="robots" content="noindex, nofollow">
<link rel="stylesheet" href="/static/admin/css/normalize.css">
<link rel="stylesheet" href="/static/admin/css/grid.css">
<link rel="stylesheet" href="/static/admin/css/style.css">
<style>.btn-drop.open > .dropdown-menu {{ display: block; }}</style>
</head><body>
{nav}
<div class="main"><div class="{container}">
<div class="typecho-page-title"><h2>{title}{add_html}</h2></div>
{main}
</div></div>
{foot}
<script>{js}</script>
</body></html>"#,
        site_title = html_escape(site.get_or("title", "Tiebidli")),
        nav = menu(user, key),
        foot = footer(),
        js = ADMIN_JS,
    );
    html_ok(html)
}

fn guest(site: &Site, title: &str, class: &str, body: &str) -> Response {
    html_ok(format!(
        r#"<!DOCTYPE HTML>
<html><head>
<meta charset="UTF-8">
<meta name="renderer" content="webkit">
<meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no">
<title>{title} - {site_title} - Powered by Typecho</title>
<meta name="robots" content="noindex, nofollow">
<link rel="stylesheet" href="/static/admin/css/normalize.css">
<link rel="stylesheet" href="/static/admin/css/grid.css">
<link rel="stylesheet" href="/static/admin/css/style.css">
</head><body class="{class}">{body}{foot}</body></html>"#,
        site_title = html_escape(site.get_or("title", "Tiebidli")),
        foot = footer(),
    ))
}

fn footer() -> &'static str {
    r#"<div class="typecho-foot" role="contentinfo"><div class="copyright">
<a href="https://typecho.org" class="i-logo-s">Typecho</a>
<p>由 <a href="https://typecho.org">Typecho</a> 强力驱动, 版本 1.2.1</p>
</div><nav class="resource">
<a href="https://docs.typecho.org">帮助文档</a> &bull;
<a href="https://forum.typecho.org">支持论坛</a> &bull;
<a href="https://github.com/typecho/typecho/issues">报告错误</a> &bull;
<a href="https://typecho.org/download">资源下载</a>
</nav></div>"#
}

fn menu(user: &SessionUser, key: &str) -> String {
    let mut groups = Vec::new();
    groups.push(("控制台", "/admin", vec![("概要", "/admin", "dashboard")]));
    let mut write = Vec::new();
    if user.pass("contributor") {
        write.push(("撰写文章", "/admin/posts/new", "write-post"));
    }
    if user.pass("editor") {
        write.push(("创建页面", "/admin/pages/new", "write-page"));
    }
    if !write.is_empty() {
        groups.push(("撰写", write[0].1, write));
    }
    let mut manage = vec![("文章", "/admin/posts", "posts")];
    if user.pass("editor") {
        manage.push(("独立页面", "/admin/pages", "pages"));
    }
    manage.push(("评论", "/admin/comments", "comments"));
    if user.pass("editor") {
        manage.push(("分类", "/admin/categories", "categories"));
        manage.push(("标签", "/admin/tags", "tags"));
        manage.push(("文件", "/admin/media", "media"));
    }
    if user.pass("administrator") {
        manage.push(("用户", "/admin/users", "users"));
    }
    groups.push(("管理", "/admin/posts", manage));
    if user.pass("administrator") {
        groups.push((
            "设置",
            "/admin/settings/general",
            vec![
                ("基本", "/admin/settings/general", "general"),
                ("评论", "/admin/settings/discussion", "discussion"),
                ("阅读", "/admin/settings/reading", "reading"),
                ("永久链接", "/admin/settings/permalink", "permalink"),
            ],
        ));
    }
    let mut html = String::from(
        r#"<div class="typecho-head-nav clearfix" role="navigation"><button class="menu-bar" type="button">菜单</button><nav id="typecho-nav-list">"#,
    );
    for (label, href, items) in &groups {
        let focus = if items.iter().any(|(_, _, k)| *k == key) { " focus" } else { "" };
        html.push_str(&format!(
            "<ul class=\"root{focus}\"><li class=\"parent\"><a href=\"{href}\">{label}</a></li><ul class=\"child\">"
        ));
        for (i, (name, url, k)) in items.iter().enumerate() {
            let mut class = String::new();
            if *k == key {
                class.push_str("focus");
            }
            if i + 1 == items.len() {
                if !class.is_empty() {
                    class.push(' ');
                }
                class.push_str("last");
            }
            let class = if class.is_empty() { String::new() } else { format!(" class=\"{class}\"") };
            html.push_str(&format!("<li{class}><a href=\"{url}\">{name}</a></li>"));
        }
        html.push_str("</ul></ul>");
    }
    html.push_str("</nav><div class=\"operate\">");
    html.push_str(&format!(
        "<a class=\"author\" href=\"/admin/users\">{}</a>",
        html_escape(&user.label())
    ));
    html.push_str("<a class=\"exit\" href=\"/admin/logout\">登出</a><a href=\"/\">网站</a></div></div>");
    html
}

fn pager_html(page: usize, pages: usize, base: &str, q: &HashMap<String, String>) -> String {
    if pages <= 1 {
        return String::new();
    }
    let mut html = String::from("<ul class=\"typecho-pager\">");
    for n in 1..=pages {
        let current = if n == page { " class=\"current\"" } else { "" };
        let mut query = q.clone();
        query.insert("page".into(), n.to_string());
        let qs: String = query.iter().map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v))).collect::<Vec<_>>().join("&");
        html.push_str(&format!("<li{current}><a href=\"{base}?{qs}\">{n}</a></li>"));
    }
    html.push_str("</ul>");
    html
}

const ADMIN_JS: &str = r##"
document.querySelectorAll('.btn-drop').forEach(function (drop) {
  var btn = drop.querySelector('.dropdown-toggle');
  if (btn) btn.addEventListener('click', function (e) {
    e.preventDefault();
    e.stopPropagation();
    drop.classList.toggle('open');
  });
  drop.querySelectorAll('.dropdown-menu a').forEach(function (a) {
    a.addEventListener('click', function (e) {
      var form = document.querySelector('form.operate-form');
      if (!form) return;
      e.preventDefault();
      e.stopPropagation();
      var lang = a.getAttribute('lang');
      if (lang && !window.confirm(lang)) return;
      form.action = a.getAttribute('href');
      form.submit();
    });
  });
});
document.addEventListener('click', function () {
  document.querySelectorAll('.btn-drop.open').forEach(function (d) { d.classList.remove('open'); });
});
document.querySelectorAll('.typecho-table-select-all').forEach(function (box) {
  box.addEventListener('change', function () {
    document.querySelectorAll('form.operate-form tbody input[type=checkbox]').forEach(function (c) {
      c.checked = box.checked;
      var tr = c.closest('tr');
      if (tr) tr.classList.toggle('checked', c.checked);
    });
  });
});
document.querySelectorAll("a[href^='#']").forEach(function (a) {
  a.addEventListener('click', function (e) {
    var id = a.getAttribute('href').slice(1);
    var panel = document.getElementById(id);
    if (!panel || !panel.classList.contains('tab-content')) return;
    e.preventDefault();
    var ul = a.closest('ul');
    if (ul) ul.querySelectorAll('li').forEach(function (li) { li.classList.remove('active'); });
    if (a.parentElement) a.parentElement.classList.add('active');
    document.querySelectorAll('.tab-content').forEach(function (p) { p.classList.add('hidden'); });
    panel.classList.remove('hidden');
  });
});
var advance = document.getElementById('advance-panel-btn');
if (advance) advance.addEventListener('click', function () {
  var p = document.getElementById('advance-panel');
  if (!p) return;
  p.style.display = p.style.display === 'block' ? 'none' : 'block';
});
var vis = document.getElementById('visibility');
if (vis) vis.addEventListener('change', function () {
  var pwd = document.getElementById('post-password');
  if (pwd) pwd.classList.toggle('hidden', vis.value !== 'password');
});
var bar = document.querySelector('button.menu-bar');
if (bar) bar.addEventListener('click', function () { bar.classList.toggle('focus'); });
document.querySelectorAll('a[lang]').forEach(function (a) {
  if (a.closest('.dropdown-menu')) return;
  a.addEventListener('click', function (e) {
    if (!window.confirm(a.getAttribute('lang'))) e.preventDefault();
  });
});
"##;

const TIMEZONES: &[(i64, &str)] = &[
    (0, "格林威治(子午线)标准时间 (GMT)"),
    (3600, "中欧标准时间 (GMT +1)"),
    (7200, "东欧标准时间 (GMT +2)"),
    (10800, "莫斯科时间 (GMT +3)"),
    (14400, "第比利斯时间 (GMT +4)"),
    (18000, "新德里时间 (GMT +5)"),
    (21600, "科伦坡时间 (GMT +6)"),
    (25200, "曼谷雅加达 (GMT +7)"),
    (28800, "北京时间 (GMT +8)"),
    (32400, "东京平壤时间 (GMT +9)"),
    (36000, "悉尼关岛时间 (GMT +10)"),
    (39600, "所罗门群岛 (GMT +11)"),
    (43200, "惠灵顿时间 (GMT +12)"),
    (-3600, "佛德尔群岛 (GMT -1)"),
    (-7200, "大西洋中部时间 (GMT -2)"),
    (-10800, "布宜诺斯艾利斯 (GMT -3)"),
    (-14400, "智利巴西 (GMT -4)"),
    (-18000, "纽约渥太华 (GMT -5)"),
    (-21600, "墨西哥城时间 (GMT -6)"),
    (-25200, "美国丹佛时间 (GMT -7)"),
    (-28800, "美国旧金山时间 (GMT -8)"),
    (-32400, "阿拉斯加时间 (GMT -9)"),
    (-36000, "夏威夷群岛 (GMT -10)"),
    (-39600, "东萨摩亚群岛 (GMT -11)"),
    (-43200, "艾尼威托克岛 (GMT -12)"),
];

async fn need(state: &AppState, headers: &HeaderMap, group: &str) -> Result<(Site, SessionUser, String), ()> {
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
    for (k, v) in split_form(raw) {
        map.insert(k, v);
    }
    map
}

fn repeated(raw: &str) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for (k, v) in split_form(raw) {
        map.entry(k).or_default().push(v);
    }
    map
}

fn split_form(raw: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let k = decode(k);
        if !k.is_empty() {
            out.push((k, decode(v)));
        }
    }
    out
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
        if raw.len() >= 9 {
            return Some(n);
        }
    }
    for fmt in ["%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(raw, fmt) {
            return Some(dt.and_utc().timestamp() - tz);
        }
        if fmt == "%Y-%m-%d" {
            if let Ok(d) = chrono::NaiveDate::parse_from_str(raw, fmt) {
                return Some(d.and_hms_opt(0, 0, 0)?.and_utc().timestamp() - tz);
            }
        }
    }
    None
}

fn date_word(ts: i64, tz: i64) -> String {
    crate::text::format_php_date("Y-m-d H:i", ts, tz)
}

fn date_short(ts: i64, tz: i64) -> String {
    crate::text::format_php_date("n.j", ts, tz)
}

fn gravatar(prefix: &str, mail: &str) -> String {
    if mail.is_empty() {
        return String::new();
    }
    let hash = auth::md5_hex(mail.trim().to_ascii_lowercase().as_bytes());
    format!("<img class=\"avatar\" src=\"{prefix}{hash}?s=40&amp;d=mm\" width=\"40\" height=\"40\" alt=\"\">")
}

fn safe_next(raw: &str) -> &str {
    if raw.starts_with("/admin") && !raw.starts_with("//") && !raw.contains("://") && !raw.starts_with("/admin/login") {
        raw
    } else {
        "/admin"
    }
}
