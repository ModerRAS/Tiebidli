//! 公开页面调度。匹配顺序与 Typecho 1.2.1 的路由表一致：某条规则 404 时继续下一条。

use crate::config::Config;
use crate::db::Db;
use crate::feed::{self, FeedKind};
use crate::links::{self, category_vals};
use crate::model::{Comment, Post, Site};
use crate::render::{self, Chrome, SingleView};
use crate::repo::{ListFilter, NewComment, Stats};
use crate::text::{self, trunc_bytes};
use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, Method, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use http_body_util::BodyExt;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tower_http::services::ServeDir;

pub struct AppState {
    pub db: Db,
    pub cfg: Config,
    pub env: minijinja::Environment<'static>,
    pub theme_ready: bool,
}

impl AppState {
    pub fn new(db: Db, cfg: Config) -> Self {
        let theme_ready = cfg.theme_dir.join("assets/css/handsome.min.css").is_file();
        Self {
            db,
            cfg,
            env: render::environment(),
            theme_ready,
        }
    }
}

pub fn build_router(state: Arc<AppState>) -> Router {
    let mut router = Router::new();
    if state.theme_ready {
        router = router.nest_service("/usr/themes/handsome", ServeDir::new(&state.cfg.theme_dir));
    }
    if let Some(dir) = &state.cfg.uploads_dir {
        router = router.nest_service("/usr/uploads", ServeDir::new(dir));
    }
    router
        .nest_service("/static", ServeDir::new(&state.cfg.static_dir))
        .fallback(dispatch)
        .with_state(state)
}

async fn dispatch(State(state): State<Arc<AppState>>, req: Request<Body>) -> Response {
    match dispatch_inner(&state, req).await {
        Ok(resp) => resp,
        Err(err) => {
            tracing::error!(error = %err, "request failed");
            plain(StatusCode::INTERNAL_SERVER_ERROR, "页面生成失败")
        }
    }
}

async fn dispatch_inner(state: &AppState, req: Request<Body>) -> anyhow::Result<Response> {
    let (parts, body) = req.into_parts();
    let bytes = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => Default::default(),
    };
    let form = if parts.method == Method::POST {
        form_pairs(&String::from_utf8_lossy(&bytes))
    } else {
        HashMap::new()
    };
    let query = form_pairs(parts.uri.query().unwrap_or(""));
    let cookies = cookies_from(&parts.headers);
    let path = normalize_path(parts.uri.path());
    let now = chrono::Utc::now().timestamp();
    let site = state.db.load_site(now).await?;
    if parts.method == Method::GET {
        if let Some(kw) = query.get("s").map(|s| s.trim()).filter(|s| !s.is_empty()) {
            let mut vals = HashMap::new();
            vals.insert("keywords".into(), links::enc(kw));
            let dest = site
                .routes
                .url("search", &vals)
                .unwrap_or_else(|| format!("/search/{}/", links::enc(kw)));
            return Ok(redirect(&dest, &HeaderMap::new()));
        }
    }
    let bag = ChromeBag {
        hot: state.db.hot_posts(5, now).await?,
        random_posts: state.db.random_posts(5, now).await?,
        recent: state.db.recent_comments(5, now).await?,
        stats: state.db.stats(now).await?,
    };
    let ctx = ReqCtx {
        state,
        site: &site,
        bag: &bag,
        now,
        method: &parts.method,
        form: &form,
        query: &query,
        cookies: &cookies,
        headers: &parts.headers,
        original_path: &path,
        feed: None,
    };
    match walk(&ctx, &path).await? {
        Step::Done(resp) => Ok(resp),
        Step::Next => not_found(&ctx).await,
    }
}

struct ChromeBag {
    hot: Vec<Post>,
    random_posts: Vec<Post>,
    recent: Vec<(Comment, Post)>,
    stats: Stats,
}

struct ReqCtx<'a> {
    state: &'a AppState,
    site: &'a Site,
    bag: &'a ChromeBag,
    now: i64,
    method: &'a Method,
    form: &'a HashMap<String, String>,
    query: &'a HashMap<String, String>,
    cookies: &'a HashMap<String, String>,
    headers: &'a HeaderMap,
    original_path: &'a str,
    feed: Option<FeedKind>,
}

enum Step {
    Done(Response),
    Next,
}

fn walk_boxed<'a>(
    ctx: &'a ReqCtx<'_>,
    path: &'a str,
) -> Pin<Box<dyn Future<Output = anyhow::Result<Step>> + Send + 'a>> {
    Box::pin(walk(ctx, path))
}

async fn walk(ctx: &ReqCtx<'_>, path: &str) -> anyhow::Result<Step> {
    let matches = ctx.site.routes.all_matches(path);
    for hit in matches {
        if ctx.feed.is_some() && matches!(hit.name.as_str(), "feed" | "do" | "feedback") {
            continue;
        }
        match handle(ctx, &hit.name, &hit.params).await? {
            Step::Next => continue,
            done => return Ok(done),
        }
    }
    if ctx.feed.is_none() {
        if let Some(slug) = archives_slug(path) {
            if let Some(post) = ctx.state.db.single_by_slug(&slug, "post", ctx.now).await? {
                return show_single(ctx, &post, SingleExtra::default()).await;
            }
        }
    }
    Ok(Step::Next)
}

async fn handle(
    ctx: &ReqCtx<'_>,
    name: &str,
    params: &HashMap<String, String>,
) -> anyhow::Result<Step> {
    match name {
        "index" | "index_page" => index_like(ctx, name, params).await,
        "archive" | "archive_page" => {
            let front = ctx.site.get("frontPage");
            if !front.is_empty() && front != "recent" && ctx.site.flag("frontArchive", false) {
                let page = if name == "archive" {
                    1
                } else {
                    page_of(params)
                };
                show_list(
                    ctx,
                    ListFilter::Index,
                    page,
                    ctx.site.get_or("title", "Tiebidli").to_string(),
                    ctx.site.theme_str("Indexwords"),
                    "archive",
                    "archive_page",
                    HashMap::new(),
                    true,
                )
                .await
            } else {
                Ok(Step::Next)
            }
        }
        "post" => routed_single(ctx, "post", params).await,
        "page" => routed_single(ctx, "page", params).await,
        "attachment" => routed_single(ctx, "attachment", params).await,
        "category" | "category_page" => category_like(ctx, name, params).await,
        "tag" | "tag_page" => tag_like(ctx, name, params).await,
        "author" | "author_page" => author_like(ctx, name, params).await,
        "search" | "search_page" => search_like(ctx, name, params).await,
        "archive_year" | "archive_year_page" | "archive_month" | "archive_month_page"
        | "archive_day" | "archive_day_page" => date_like(ctx, name, params).await,
        "comment_page" => {
            let perm = params.get("permalink").cloned().unwrap_or_default();
            let path = leading_slash(&perm);
            match find_single(ctx, &path).await? {
                Some(post) => show_single(ctx, &post, SingleExtra::default()).await,
                None => Ok(Step::Next),
            }
        }
        "feed" => feed_like(ctx, params).await,
        "feedback" => {
            if params.get("type").map(String::as_str) != Some("comment")
                || *ctx.method != Method::POST
            {
                return Ok(Step::Next);
            }
            let perm = params.get("permalink").cloned().unwrap_or_default();
            post_comment(ctx, &leading_slash(&perm)).await
        }
        _ => Ok(Step::Next),
    }
}

async fn index_like(
    ctx: &ReqCtx<'_>,
    name: &str,
    params: &HashMap<String, String>,
) -> anyhow::Result<Step> {
    if ctx.feed.is_none() {
        let front = ctx.site.get("frontPage");
        if let Some(raw) = front.strip_prefix("page:") {
            let cid: i64 = raw.parse().unwrap_or(0);
            if let Some(post) = ctx.state.db.single_by_cid(cid, "page", ctx.now).await? {
                return show_single(ctx, &post, SingleExtra::default()).await;
            }
            return Ok(Step::Next);
        }
        if front.starts_with("file:") {
            return not_found(ctx).await.map(Step::Done);
        }
    }
    let page = if name == "index" { 1 } else { page_of(params) };
    show_list(
        ctx,
        ListFilter::Index,
        page,
        ctx.site.get_or("title", "Tiebidli").to_string(),
        ctx.site.theme_str("Indexwords"),
        "index",
        "index_page",
        HashMap::new(),
        true,
    )
    .await
}

async fn routed_single(
    ctx: &ReqCtx<'_>,
    kind: &str,
    params: &HashMap<String, String>,
) -> anyhow::Result<Step> {
    let Some(post) = load_routed(ctx, kind, params).await? else {
        return Ok(Step::Next);
    };
    show_single(ctx, &post, SingleExtra::default()).await
}

async fn load_routed(
    ctx: &ReqCtx<'_>,
    kind: &str,
    params: &HashMap<String, String>,
) -> anyhow::Result<Option<Post>> {
    let post = if let Some(cid) = params.get("cid").and_then(|s| s.parse::<i64>().ok()) {
        ctx.state.db.single_by_cid(cid, kind, ctx.now).await?
    } else if let Some(slug) = params.get("slug") {
        ctx.state.db.single_by_slug(slug, kind, ctx.now).await?
    } else {
        None
    };
    if let (Some(post), Some(slug)) = (post.as_ref(), params.get("slug")) {
        if &post.slug != slug {
            return Ok(None);
        }
    }
    if let (Some(post), Some(cid)) = (
        post.as_ref(),
        params.get("cid").and_then(|s| s.parse::<i64>().ok()),
    ) {
        if post.cid != cid {
            return Ok(None);
        }
    }
    if let Some(post) = post.as_ref() {
        if let Some(year) = params.get("year").and_then(|s| s.parse::<i32>().ok()) {
            let month = params
                .get("month")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let day = params.get("day").and_then(|s| s.parse().ok()).unwrap_or(0);
            if let Some((start, end)) = text::calendar_range(year, month, day, ctx.site.timezone())
            {
                if post.created < start || post.created >= end {
                    return Ok(None);
                }
            } else {
                return Ok(None);
            }
        }
        if let Some(cat) = params.get("category") {
            let slug = post
                .categories
                .first()
                .map(|c| c.slug.as_str())
                .unwrap_or("");
            if cat != slug && cat != &links::enc(slug) {
                return Ok(None);
            }
        }
        if let Some(dir) = params.get("directory") {
            let vals = links::post_values(ctx.site, post);
            let raw = vals.get("directory_raw").map(String::as_str).unwrap_or("");
            let encoded = vals.get("directory").map(String::as_str).unwrap_or("");
            if dir != raw && dir != encoded {
                return Ok(None);
            }
        }
    }
    Ok(post)
}

async fn find_single(ctx: &ReqCtx<'_>, path: &str) -> anyhow::Result<Option<Post>> {
    let path = normalize_path(path);
    for hit in ctx.site.routes.all_matches(&path) {
        let kind = match hit.name.as_str() {
            "post" => "post",
            "page" => "page",
            "attachment" => "attachment",
            _ => continue,
        };
        if let Some(post) = load_routed(ctx, kind, &hit.params).await? {
            return Ok(Some(post));
        }
    }
    if let Some(slug) = archives_slug(&path) {
        return Ok(ctx.state.db.single_by_slug(&slug, "post", ctx.now).await?);
    }
    Ok(None)
}

async fn category_like(
    ctx: &ReqCtx<'_>,
    name: &str,
    params: &HashMap<String, String>,
) -> anyhow::Result<Step> {
    let Some(meta) = resolve_meta(ctx, "category", params).await? else {
        return Ok(Step::Next);
    };
    if let Some(dir) = params.get("directory") {
        let vals = category_vals(ctx.site, &meta);
        let raw = vals.get("directory_raw").map(String::as_str).unwrap_or("");
        let encoded = vals.get("directory").map(String::as_str).unwrap_or("");
        if dir != raw && dir != encoded {
            return Ok(Step::Next);
        }
    }
    let mids = links::child_mids(&ctx.site.categories, meta.mid);
    let page = if name.ends_with("_page") {
        page_of(params)
    } else {
        1
    };
    let vals = category_vals(ctx.site, &meta);
    let heading = meta.name.clone();
    let sub = meta.description.clone();
    show_list(
        ctx,
        ListFilter::Category(mids),
        page,
        heading,
        sub,
        "category",
        "category_page",
        vals,
        true,
    )
    .await
}

async fn tag_like(
    ctx: &ReqCtx<'_>,
    name: &str,
    params: &HashMap<String, String>,
) -> anyhow::Result<Step> {
    let Some(meta) = resolve_meta(ctx, "tag", params).await? else {
        return Ok(Step::Next);
    };
    let page = if name.ends_with("_page") {
        page_of(params)
    } else {
        1
    };
    let mut vals = HashMap::new();
    vals.insert("slug".into(), links::enc(&meta.slug));
    let heading = meta.name.clone();
    let sub = meta.description.clone();
    let mid = meta.mid;
    show_list(
        ctx,
        ListFilter::Tag(mid),
        page,
        heading,
        sub,
        "tag",
        "tag_page",
        vals,
        true,
    )
    .await
}

async fn resolve_meta(
    ctx: &ReqCtx<'_>,
    kind: &str,
    params: &HashMap<String, String>,
) -> anyhow::Result<Option<crate::model::Meta>> {
    if let Some(mid) = params.get("mid").and_then(|s| s.parse::<i64>().ok()) {
        let found = match kind {
            "category" => ctx.site.categories.iter().find(|m| m.mid == mid).cloned(),
            _ => ctx.site.tags.iter().find(|m| m.mid == mid).cloned(),
        };
        if found.is_some() {
            return Ok(found);
        }
    }
    let slug = if let Some(slug) = params.get("slug") {
        slug.clone()
    } else if let Some(dir) = params.get("directory") {
        dir.split('/').next_back().unwrap_or("").to_string()
    } else {
        return Ok(None);
    };
    if slug.is_empty() {
        return Ok(None);
    }
    Ok(ctx.state.db.meta_by_slug(kind, &slug).await?)
}

async fn author_like(
    ctx: &ReqCtx<'_>,
    name: &str,
    params: &HashMap<String, String>,
) -> anyhow::Result<Step> {
    let Some(uid) = params.get("uid").and_then(|s| s.parse::<i64>().ok()) else {
        return Ok(Step::Next);
    };
    let Some(author) = ctx.state.db.author_name(uid).await? else {
        return Ok(Step::Next);
    };
    let page = if name.ends_with("_page") {
        page_of(params)
    } else {
        1
    };
    let mut vals = HashMap::new();
    vals.insert("uid".into(), uid.to_string());
    show_list(
        ctx,
        ListFilter::Author(uid),
        page,
        author,
        String::new(),
        "author",
        "author_page",
        vals,
        true,
    )
    .await
}

async fn search_like(
    ctx: &ReqCtx<'_>,
    name: &str,
    params: &HashMap<String, String>,
) -> anyhow::Result<Step> {
    let keyword = params.get("keywords").cloned().unwrap_or_default();
    let page = if name.ends_with("_page") {
        page_of(params)
    } else {
        1
    };
    let page_size = if ctx.feed.is_some() {
        10
    } else {
        ctx.site.page_size()
    };
    let (posts, total) = ctx
        .state
        .db
        .list_posts(
            &ListFilter::Search(keyword.clone()),
            page,
            page_size,
            ctx.now,
            ctx.feed.is_some(),
        )
        .await?;
    if page > 1 && posts.is_empty() {
        return Ok(Step::Next);
    }
    if let Some(kind) = ctx.feed {
        let title = format!("包含关键字 {keyword} 的文章");
        return Ok(Step::Done(xml_response(
            kind,
            feed::render_posts(
                kind,
                ctx.site,
                &base_of(ctx),
                &title,
                ctx.site.get("description"),
                ctx.original_path,
                &posts,
                ctx.site.flag("feedFullText", true),
            ),
        )));
    }
    let chrome = chrome(ctx, ctx.cookies.clone());
    let html = render::render_search(&ctx.state.env, &chrome, &keyword, &posts, page, total)?;
    Ok(Step::Done(html_response(
        StatusCode::OK,
        html,
        &HeaderMap::new(),
    )))
}

async fn date_like(
    ctx: &ReqCtx<'_>,
    name: &str,
    params: &HashMap<String, String>,
) -> anyhow::Result<Step> {
    let Some(year) = params.get("year").and_then(|s| s.parse::<i32>().ok()) else {
        return Ok(Step::Next);
    };
    let month = if name.contains("month") || name.contains("day") {
        params
            .get("month")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    } else {
        0
    };
    let day = if name.contains("day") {
        params.get("day").and_then(|s| s.parse().ok()).unwrap_or(0)
    } else {
        0
    };
    let Some((start, end)) = text::calendar_range(year, month, day, ctx.site.timezone()) else {
        return Ok(Step::Next);
    };
    let page = if name.ends_with("_page") {
        page_of(params)
    } else {
        1
    };
    let (base_name, page_name) = if day > 0 {
        ("archive_day", "archive_day_page")
    } else if month > 0 {
        ("archive_month", "archive_month_page")
    } else {
        ("archive_year", "archive_year_page")
    };
    let mut vals = HashMap::new();
    vals.insert("year".into(), format!("{year:04}"));
    if month > 0 {
        vals.insert("month".into(), format!("{month:02}"));
    }
    if day > 0 {
        vals.insert("day".into(), format!("{day:02}"));
    }
    let heading = if day > 0 {
        format!("{year}年{month}月{day}日")
    } else if month > 0 {
        format!("{year}年{month}月")
    } else {
        format!("{year}年")
    };
    show_list(
        ctx,
        ListFilter::Date { start, end },
        page,
        heading,
        String::new(),
        base_name,
        page_name,
        vals,
        true,
    )
    .await
}

async fn show_list(
    ctx: &ReqCtx<'_>,
    filter: ListFilter,
    page: i64,
    heading: String,
    sub: String,
    base_name: &str,
    page_name: &str,
    vals: HashMap<String, String>,
    show_cloud: bool,
) -> anyhow::Result<Step> {
    let page_size = if ctx.feed.is_some() {
        10
    } else {
        ctx.site.page_size()
    };
    let (posts, total) = ctx
        .state
        .db
        .list_posts(&filter, page, page_size, ctx.now, ctx.feed.is_some())
        .await?;
    if page > 1 && posts.is_empty() {
        return Ok(Step::Next);
    }
    if let Some(kind) = ctx.feed {
        return Ok(Step::Done(xml_response(
            kind,
            feed::render_posts(
                kind,
                ctx.site,
                &base_of(ctx),
                &heading,
                if sub.is_empty() {
                    ctx.site.get("description")
                } else {
                    sub.as_str()
                },
                ctx.original_path,
                &posts,
                ctx.site.flag("feedFullText", true),
            ),
        )));
    }
    let chrome = chrome(ctx, ctx.cookies.clone());
    let html = render::render_list(
        &ctx.state.env,
        &chrome,
        &heading,
        &sub,
        &posts,
        page,
        total,
        base_name,
        page_name,
        &vals,
        show_cloud,
    )?;
    Ok(Step::Done(html_response(
        StatusCode::OK,
        html,
        &HeaderMap::new(),
    )))
}

struct SingleExtra<'a> {
    password_error: &'a str,
    comment_error: &'a str,
    cookies: Option<HashMap<String, String>>,
    remember_author: Option<&'a str>,
    remember_mail: Option<&'a str>,
    remember_url: Option<&'a str>,
    remember_text: Option<&'a str>,
    status: Option<StatusCode>,
    extra: HeaderMap,
}

impl Default for SingleExtra<'static> {
    fn default() -> Self {
        Self {
            password_error: "",
            comment_error: "",
            cookies: None,
            remember_author: None,
            remember_mail: None,
            remember_url: None,
            remember_text: None,
            status: None,
            extra: HeaderMap::new(),
        }
    }
}

async fn show_single(
    ctx: &ReqCtx<'_>,
    post: &Post,
    extra: SingleExtra<'_>,
) -> anyhow::Result<Step> {
    if let Some(kind) = ctx.feed {
        if post.has_password() || !post.allow_feed {
            return Ok(Step::Next);
        }
        let comments = ctx.state.db.comments_for(post.cid, "", "", true).await?;
        let rows: Vec<(Comment, Post)> = comments
            .into_iter()
            .take(10)
            .map(|c| (c, post.clone()))
            .collect();
        let title = format!(
            "{} - {} 的评论",
            ctx.site.get_or("title", "Tiebidli"),
            post.title
        );
        let xml = feed::render_comments(
            kind,
            ctx.site,
            &base_of(ctx),
            &title,
            ctx.original_path,
            &rows,
            ctx.site.flag("commentsMarkdown", false),
        );
        return Ok(Step::Done(xml_response(kind, xml)));
    }

    let mut cookies = extra.cookies.unwrap_or_else(|| ctx.cookies.clone());
    let mut headers = extra.extra;
    let mut password_error = extra.password_error.to_string();
    if *ctx.method == Method::POST && extra.comment_error.is_empty() && password_error.is_empty() {
        if let Some(pw) = ctx.form.get("protectPassword") {
            let cid = ctx
                .form
                .get("protectCID")
                .and_then(|s| s.parse().ok())
                .unwrap_or(post.cid);
            if cid == post.cid {
                let key = format!("protectPassword_{cid}");
                cookies.insert(key.clone(), pw.clone());
                push_cookie(&mut headers, &key, pw, None);
                if post.has_password() && pw != &post.password {
                    password_error = "对不起,您输入的密码错误".into();
                }
            }
        }
    }
    let locked = post.has_password()
        && cookies
            .get(&format!("protectPassword_{}", post.cid))
            .map(|v| v != &post.password)
            .unwrap_or(true);
    let author = extra
        .remember_author
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            cookies
                .get("__typecho_remember_author")
                .cloned()
                .unwrap_or_default()
        });
    let mail = extra
        .remember_mail
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            cookies
                .get("__typecho_remember_mail")
                .cloned()
                .unwrap_or_default()
        });
    let url = extra
        .remember_url
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            cookies
                .get("__typecho_remember_url")
                .cloned()
                .unwrap_or_default()
        });
    let text = extra
        .remember_text
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            cookies
                .get("__typecho_remember_text")
                .cloned()
                .unwrap_or_default()
        });
    let comment_only = ctx.site.flag("commentsShowCommentOnly", false);
    let mut comments = if locked {
        Vec::new()
    } else {
        ctx.state
            .db
            .comments_for(post.cid, &author, &mail, comment_only)
            .await?
    };
    if comments.len() > 300 {
        comments.truncate(300);
    }
    let (newer, older) = if post.kind == "post" {
        ctx.state.db.neighbors(post, ctx.now).await?
    } else {
        (None, None)
    };
    let chrome = chrome(ctx, cookies);
    let reply_to = ctx
        .query
        .get("replyTo")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let view = SingleView {
        post,
        locked,
        password_error: &password_error,
        comment_error: extra.comment_error,
        comments: &comments,
        newer: newer.as_ref(),
        older: older.as_ref(),
        remember_author: &author,
        remember_mail: &mail,
        remember_url: &url,
        remember_text: &text,
        reply_to,
    };
    let html = render::render_single(&ctx.state.env, &chrome, &view)?;
    let status = extra.status.unwrap_or(if locked {
        StatusCode::FORBIDDEN
    } else {
        StatusCode::OK
    });
    Ok(Step::Done(html_response(status, html, &headers)))
}

async fn feed_like(ctx: &ReqCtx<'_>, params: &HashMap<String, String>) -> anyhow::Result<Step> {
    let raw = params.get("feed").cloned().unwrap_or_default();
    let (kind, rest) = split_feed(&raw);
    let rest = normalize_path(&leading_slash(rest.trim()));
    if rest == "/comments" {
        let rows = ctx.state.db.recent_comments(10, ctx.now).await?;
        let title = format!("{} 的评论", ctx.site.get_or("title", "Tiebidli"));
        let xml = feed::render_comments(
            kind,
            ctx.site,
            &base_of(ctx),
            &title,
            ctx.original_path,
            &rows,
            ctx.site.flag("commentsMarkdown", false),
        );
        return Ok(Step::Done(xml_response(kind, xml)));
    }
    let inner = ReqCtx {
        feed: Some(kind),
        original_path: ctx.original_path,
        ..*ctx
    };
    let target = if rest == "/" { "/" } else { rest.as_str() };
    match walk_boxed(&inner, target).await? {
        Step::Done(resp) => Ok(Step::Done(resp)),
        Step::Next => not_found(ctx).await.map(Step::Done),
    }
}

async fn post_comment(ctx: &ReqCtx<'_>, path: &str) -> anyhow::Result<Step> {
    let Some(post) = find_single(ctx, path).await? else {
        return Ok(Step::Next);
    };
    if !post.allow_comment
        || !comments_open(ctx.site, &post, ctx.now)
        || !unlocked(ctx.cookies, &post)
    {
        return comment_fail(
            ctx,
            &post,
            "对不起,此内容的反馈被禁止.",
            StatusCode::FORBIDDEN,
        )
        .await;
    }
    if ctx
        .form
        .get("tiebidli_hp")
        .is_some_and(|s| !s.trim().is_empty())
    {
        return Ok(Step::Done(redirect(
            &links::content_path(ctx.site, &post),
            &HeaderMap::new(),
        )));
    }
    let author = trunc_chars(ctx.form.get("author").map(|s| s.trim()).unwrap_or(""), 150);
    let mail = trunc_chars(ctx.form.get("mail").map(|s| s.trim()).unwrap_or(""), 150);
    let mut url = ctx
        .form
        .get("url")
        .map(|s| s.trim())
        .unwrap_or("")
        .to_string();
    if !url.is_empty() && !url.starts_with("http://") && !url.starts_with("https://") {
        url = format!("http://{url}");
    }
    url = trunc_chars(&url, 255);
    let text = ctx
        .form
        .get("text")
        .map(|s| s.trim())
        .unwrap_or("")
        .to_string();
    let mut fail: Option<(&str, StatusCode)> = None;
    if author.is_empty() {
        fail = Some(("必须填写用户名", StatusCode::BAD_REQUEST));
    } else if !xss_ok(&author) {
        fail = Some(("请不要在用户名中使用特殊字符", StatusCode::BAD_REQUEST));
    } else if ctx.state.db.user_name_taken(&author).await? {
        fail = Some((
            "您所使用的用户名已经被注册,请登录后再次提交",
            StatusCode::BAD_REQUEST,
        ));
    } else if ctx.site.flag("commentsRequireMail", true) && mail.is_empty() {
        fail = Some(("必须填写电子邮箱地址", StatusCode::BAD_REQUEST));
    } else if !mail.is_empty() && !looks_like_email(&mail) {
        fail = Some(("邮箱地址不合法", StatusCode::BAD_REQUEST));
    } else if ctx.site.flag("commentsRequireURL", false) && url.is_empty() {
        fail = Some(("必须填写个人主页", StatusCode::BAD_REQUEST));
    } else if !url.is_empty() && !looks_like_url(&url) {
        fail = Some(("个人主页地址格式错误", StatusCode::BAD_REQUEST));
    } else if text.is_empty() {
        fail = Some(("必须填写评论内容", StatusCode::BAD_REQUEST));
    }
    let parent: i64 = ctx
        .form
        .get("parent")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if fail.is_none() && parent != 0 {
        let threaded = ctx.site.flag("commentsThreaded", true);
        let row = ctx.state.db.comment_parent(parent).await?;
        if !threaded || row.is_none_or(|(_, cid)| cid != post.cid) {
            fail = Some(("父级评论不存在", StatusCode::BAD_REQUEST));
        }
    }
    if fail.is_none() && ctx.site.flag("commentsCheckReferer", true) {
        let referer = ctx
            .headers
            .get(header::REFERER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let permalink = links::content_path(ctx.site, &post);
        let front = ctx.site.get("frontPage") == format!("page:{}", post.cid);
        if !referer_ok(referer, &permalink, &base_of(ctx), front) {
            fail = Some(("评论来源页错误.", StatusCode::FORBIDDEN));
        }
    }
    let ip = client_ip(ctx.headers);
    if fail.is_none() && ctx.site.flag("commentsPostIntervalEnable", true) {
        let interval = ctx.site.int("commentsPostInterval", 60);
        if interval > 0 {
            let latest = ctx.state.db.latest_comment_at(post.cid, &ip).await?;
            if latest > 0 && ctx.now > latest && ctx.now - latest < interval {
                fail = Some((
                    "对不起, 您的发言过于频繁, 请稍侯再次发布.",
                    StatusCode::FORBIDDEN,
                ));
            }
        }
    }
    if let Some((msg, status)) = fail {
        return comment_fail_fields(ctx, &post, msg, status, &author, &mail, &url, &text).await;
    }
    let mut status = if ctx.site.flag("commentsRequireModeration", false) {
        "waiting"
    } else {
        "approved"
    };
    if status == "approved" && ctx.site.flag("commentsWhitelist", false) {
        let prior = ctx.state.db.approved_by_author(&author, &mail).await?;
        if prior == 0 {
            status = "waiting";
        }
    }
    if ip_blocked(ctx.site, &ip) || stop_word(ctx.site, &text) {
        status = "spam";
    }
    let agent = trunc_chars(
        ctx.headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        511,
    );
    let id = ctx
        .state
        .db
        .insert_comment(&NewComment {
            cid: post.cid,
            created: ctx.now,
            author: author.clone(),
            author_id: 0,
            owner_id: post.author_id,
            mail: mail.clone(),
            url: url.clone(),
            ip: trunc_bytes(&ip, 64),
            agent,
            text,
            status: status.into(),
            parent,
        })
        .await?;
    let mut headers = HeaderMap::new();
    remember_cookies(&mut headers, &author, &mail, &url, None);
    let permalink = links::content_path(ctx.site, &post);
    let dest = if status == "spam" {
        permalink
    } else {
        format!("{permalink}#comment-{id}")
    };
    Ok(Step::Done(redirect(&dest, &headers)))
}

async fn comment_fail(
    ctx: &ReqCtx<'_>,
    post: &Post,
    msg: &str,
    status: StatusCode,
) -> anyhow::Result<Step> {
    comment_fail_fields(ctx, post, msg, status, "", "", "", "").await
}

async fn comment_fail_fields(
    ctx: &ReqCtx<'_>,
    post: &Post,
    msg: &str,
    status: StatusCode,
    author: &str,
    mail: &str,
    url: &str,
    text: &str,
) -> anyhow::Result<Step> {
    let mut headers = HeaderMap::new();
    if !author.is_empty() || !text.is_empty() {
        remember_cookies(&mut headers, author, mail, url, Some(text));
    }
    show_single(
        ctx,
        post,
        SingleExtra {
            password_error: "",
            comment_error: msg,
            cookies: None,
            remember_author: nonempty_ref(author),
            remember_mail: nonempty_ref(mail),
            remember_url: nonempty_ref(url),
            remember_text: nonempty_ref(text),
            status: Some(status),
            extra: headers,
        },
    )
    .await
}

fn nonempty_ref(s: &str) -> Option<&str> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn comments_open(site: &Site, post: &Post, now: i64) -> bool {
    if site.flag("commentsAutoClose", false) {
        let timeout = site.int("commentsPostTimeout", 24 * 3600 * 30);
        if timeout > 0 && now - post.created > timeout {
            return false;
        }
    }
    true
}

fn unlocked(cookies: &HashMap<String, String>, post: &Post) -> bool {
    !post.has_password()
        || cookies
            .get(&format!("protectPassword_{}", post.cid))
            .is_some_and(|v| v == &post.password)
}

fn stop_word(site: &Site, text: &str) -> bool {
    site.get("commentsStopWords").lines().any(|line| {
        let line = line.trim();
        !line.is_empty() && text.contains(line)
    })
}

fn ip_blocked(site: &Site, ip: &str) -> bool {
    site.get("commentsIpBlackList").lines().any(|line| {
        let line = line.trim();
        !line.is_empty() && line == ip
    })
}

fn referer_ok(referer: &str, permalink_path: &str, site_base: &str, is_front: bool) -> bool {
    let Ok(referer) = url::Url::parse(referer) else {
        return false;
    };
    let Ok(base) = url::Url::parse(site_base) else {
        return false;
    };
    if referer.host_str() != base.host_str() {
        return false;
    }
    let rp = referer.path().trim_end_matches('/');
    let pp = permalink_path.trim_end_matches('/');
    if rp.starts_with(pp) {
        return true;
    }
    is_front && rp.is_empty()
}

fn looks_like_email(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !s
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\''))
}

fn looks_like_url(s: &str) -> bool {
    matches!(url::Url::parse(s), Ok(u) if u.scheme() == "http" || u.scheme() == "https")
        && !s.chars().any(char::is_whitespace)
}

fn xss_ok(s: &str) -> bool {
    !s.chars().any(|c| {
        matches!(c, '(' | ')' | '\\' | '"' | '<' | '>' | '\r' | '\n' | '\t') || (c as u32) < 0x20
    })
}

fn client_ip(headers: &HeaderMap) -> String {
    if let Some(v) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        let ip = v.split(',').next().unwrap_or("").trim();
        if !ip.is_empty() {
            return trunc_chars(ip, 64);
        }
    }
    if let Some(v) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
        if !v.trim().is_empty() {
            return trunc_chars(v.trim(), 64);
        }
    }
    "0.0.0.0".into()
}

fn remember_cookies(
    headers: &mut HeaderMap,
    author: &str,
    mail: &str,
    url: &str,
    text: Option<&str>,
) {
    let month = 30 * 24 * 3600;
    push_cookie(headers, "__typecho_remember_author", author, Some(month));
    push_cookie(headers, "__typecho_remember_mail", mail, Some(month));
    push_cookie(headers, "__typecho_remember_url", url, Some(month));
    match text {
        Some(text) => push_cookie(headers, "__typecho_remember_text", text, None),
        None => push_cookie(headers, "__typecho_remember_text", "", Some(0)),
    }
}

fn split_feed(raw: &str) -> (FeedKind, String) {
    if raw == "/rss" || raw.starts_with("/rss/") {
        (FeedKind::Rss, raw[4..].to_string())
    } else if raw == "/atom" || raw.starts_with("/atom/") {
        (FeedKind::Atom, raw[5..].to_string())
    } else {
        (FeedKind::Rss, raw.to_string())
    }
}

fn chrome<'a>(ctx: &ReqCtx<'a>, cookies: HashMap<String, String>) -> Chrome<'a> {
    Chrome {
        cfg: &ctx.state.cfg,
        site: ctx.site,
        theme_ready: ctx.state.theme_ready,
        hot: &ctx.bag.hot,
        random_posts: &ctx.bag.random_posts,
        recent: &ctx.bag.recent,
        stats: &ctx.bag.stats,
        now: ctx.now,
        cookies,
    }
}

async fn not_found(ctx: &ReqCtx<'_>) -> anyhow::Result<Response> {
    let chrome = chrome(ctx, ctx.cookies.clone());
    let html = render::render_404(&ctx.state.env, &chrome)?;
    Ok(html_response(
        StatusCode::NOT_FOUND,
        html,
        &HeaderMap::new(),
    ))
}

fn base_of(ctx: &ReqCtx<'_>) -> String {
    links::site_base(&ctx.state.cfg.site_url, ctx.site)
}

fn page_of(params: &HashMap<String, String>) -> i64 {
    params
        .get("page")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1)
        .max(1)
}

fn leading_slash(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else if path.is_empty() {
        "/".into()
    } else {
        format!("/{path}")
    }
}

fn archives_slug(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/archives/")?;
    if rest.is_empty() || rest.contains('/') {
        return None;
    }
    let rest = rest.strip_suffix(".html").unwrap_or(rest);
    if rest.is_empty() || rest.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(rest.to_string())
}

pub fn normalize_path(raw: &str) -> String {
    let raw = raw.split('?').next().unwrap_or(raw);
    let path = raw
        .strip_prefix("/index.php")
        .map(|rest| if rest.is_empty() { "/" } else { rest })
        .unwrap_or(raw);
    let decoded = path
        .split('/')
        .map(|seg| {
            urlencoding::decode(seg)
                .map(|s| s.into_owned())
                .unwrap_or_else(|_| seg.to_string())
        })
        .collect::<Vec<_>>()
        .join("/");
    let decoded = if decoded.len() > 1 {
        decoded.trim_end_matches('/').to_string()
    } else {
        decoded
    };
    if decoded.is_empty() {
        "/".into()
    } else {
        decoded
    }
}

fn form_pairs(raw: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let k = decode_component(k);
        if k.is_empty() {
            continue;
        }
        map.insert(k, decode_component(v));
    }
    map
}

fn decode_component(s: &str) -> String {
    let spaced = s.replace('+', " ");
    urlencoding::decode(&spaced)
        .map(|c| c.into_owned())
        .unwrap_or(spaced)
}

fn cookies_from(headers: &HeaderMap) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for val in headers.get_all(header::COOKIE) {
        let Ok(s) = val.to_str() else {
            continue;
        };
        for part in s.split(';') {
            let Some((k, v)) = part.trim().split_once('=') else {
                continue;
            };
            map.insert(k.to_string(), decode_component(v));
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

fn html_response(status: StatusCode, body: String, extra: &HeaderMap) -> Response {
    let mut res = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(body))
        .unwrap_or_else(|_| plain(StatusCode::INTERNAL_SERVER_ERROR, "页面生成失败"));
    for (k, v) in extra.iter() {
        res.headers_mut().append(k, v.clone());
    }
    res
}

fn xml_response(kind: FeedKind, body: String) -> Response {
    let ct = match kind {
        FeedKind::Rss => "application/rss+xml; charset=utf-8",
        FeedKind::Atom => "application/atom+xml; charset=utf-8",
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, ct)
        .body(Body::from(body))
        .unwrap_or_else(|_| plain(StatusCode::INTERNAL_SERVER_ERROR, "订阅生成失败"))
}

fn redirect(loc: &str, extra: &HeaderMap) -> Response {
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

fn plain(status: StatusCode, msg: &str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(msg.to_string()))
        .unwrap_or_else(|_| Response::new(Body::from(msg.to_string())))
}

fn trunc_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// `..*ctx` needs ReqCtx to be Copy. It contains references only.
impl<'a> Copy for ReqCtx<'a> {}
impl<'a> Clone for ReqCtx<'a> {
    fn clone(&self) -> Self {
        *self
    }
}
