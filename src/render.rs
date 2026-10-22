use crate::config::Config;
use crate::links::{self, abs};
use crate::model::{Comment, Meta, Post, Site};
use crate::repo::Stats;
use crate::text::{self, html_escape};
use minijinja::{context, Environment, Error};
use serde_json::{json, Value};
use std::collections::HashMap;

const PRESETS: &[(&str, &str, &str)] = &[
    ("black", "white-only", "black"),
    ("dark", "white-only", "dark"),
    ("white", "white-only", "black"),
    ("primary", "white-only", "dark"),
    ("info", "white-only", "black"),
    ("success", "white-only", "dark"),
    ("danger", "white-only", "dark"),
    ("black", "black", "white"),
    ("dark", "dark", "light"),
    ("info dker", "info dker", "light"),
    ("primary", "primary", "dark"),
    ("info dker", "info dk", "black"),
    ("success", "success", "dark"),
    ("danger", "danger", "dark"),
];

pub struct Chrome<'a> {
    pub cfg: &'a Config,
    pub site: &'a Site,
    pub theme_ready: bool,
    pub hot: &'a [Post],
    pub random_posts: &'a [Post],
    pub recent: &'a [(Comment, Post)],
    pub stats: &'a Stats,
    pub now: i64,
    pub cookies: HashMap<String, String>,
}

const EMBEDDED: &[(&str, &str)] = &[
    ("base.html", include_str!("../templates/base.html")),
    ("list.html", include_str!("../templates/list.html")),
    ("single.html", include_str!("../templates/single.html")),
    ("search.html", include_str!("../templates/search.html")),
    ("404.html", include_str!("../templates/404.html")),
];

/// `template_dir` 里的同名 html 覆盖内置模板。进程启动时读入，之后不热更新。
pub fn environment(template_dir: Option<&std::path::Path>) -> Environment<'static> {
    let mut env = Environment::new();
    env.set_auto_escape_callback(|name| {
        if name.ends_with(".html") {
            minijinja::AutoEscape::Html
        } else {
            minijinja::AutoEscape::None
        }
    });
    for (name, embedded) in EMBEDDED {
        let src: &'static str = match template_dir {
            Some(dir) => match std::fs::read_to_string(dir.join(name)) {
                Ok(text) => Box::leak(text.into_boxed_str()),
                Err(_) => embedded,
            },
            None => embedded,
        };
        env.add_template(name, src).expect(name);
    }
    env
}

pub fn render_list(
    env: &Environment<'_>,
    chrome: &Chrome<'_>,
    heading: &str,
    sub: &str,
    posts: &[Post],
    page: i64,
    total: i64,
    base_route: &str,
    page_route: &str,
    route_vals: &HashMap<String, String>,
    show_tag_cloud: bool,
) -> Result<String, Error> {
    let cards: Vec<Value> = posts.iter().map(|p| post_card(chrome, p)).collect();
    let pagination = links::pagination(page, total, chrome.site.page_size(), |n| {
        links::paged_path(chrome.site, base_route, page_route, route_vals.clone(), n)
    });
    let mut ctx = shell(
        chrome,
        &format_title(heading, chrome, page),
        sub,
        show_tag_cloud,
        &[],
    );
    ctx["heading"] = json!(heading);
    ctx["sub"] = json!(sub);
    ctx["posts"] = json!(cards);
    ctx["pagination"] = json!(pagination);
    env.get_template("list.html")?.render(ctx)
}

pub fn render_search(
    env: &Environment<'_>,
    chrome: &Chrome<'_>,
    keyword: &str,
    posts: &[Post],
    page: i64,
    total: i64,
) -> Result<String, Error> {
    let cards: Vec<Value> = posts
        .iter()
        .map(|p| {
            let mut card = post_card(chrome, p);
            let plain = text::plain_text(&p.text);
            card["summary"] = json!(text::snippet_around(&plain, keyword, 60));
            card
        })
        .collect();
    let mut vals = HashMap::new();
    vals.insert("keywords".into(), links::enc(keyword));
    let pagination = links::pagination(page, total, chrome.site.page_size(), |n| {
        links::paged_path(chrome.site, "search", "search_page", vals.clone(), n)
    });
    let title = format!("包含关键字 {keyword} 的文章");
    let mut ctx = shell(chrome, &format_title(&title, chrome, page), "", false, &[]);
    ctx["keyword"] = json!(keyword);
    ctx["count"] = json!(total);
    ctx["posts"] = json!(cards);
    ctx["pagination"] = json!(pagination);
    env.get_template("search.html")?.render(ctx)
}

pub struct SingleView<'a> {
    pub post: &'a Post,
    pub locked: bool,
    pub password_error: &'a str,
    pub comment_error: &'a str,
    pub comments: &'a [Comment],
    pub newer: Option<&'a Post>,
    pub older: Option<&'a Post>,
    pub remember_author: &'a str,
    pub remember_mail: &'a str,
    pub remember_url: &'a str,
    pub remember_text: &'a str,
    pub reply_to: i64,
    pub comment_nav: &'a str,
    pub ping_html: &'a str,
    pub attachment_html: &'a str,
}

pub fn render_single(
    env: &Environment<'_>,
    chrome: &Chrome<'_>,
    view: &SingleView<'_>,
) -> Result<String, Error> {
    let post = view.post;
    let site = chrome.site;
    let permalink = links::content_path(site, post);
    let heading = if view.locked {
        "此内容被密码保护".to_string()
    } else {
        post.title.clone()
    };
    let page_title = format_title(&heading, chrome, 1);
    let desc = if view.locked {
        "请输入密码访问".to_string()
    } else {
        text::excerpt(&post.text, 140)
    };
    let show_cloud = false;
    let post_tags: Vec<Value> = post
        .tags
        .iter()
        .map(|t| json!({"name": t.name, "url": links::tag_path(site, t)}))
        .collect();
    let mut ctx = shell(chrome, &page_title, &desc, show_cloud, &post_tags);
    let comment_label = comment_label(post, view.locked);
    let cats: Vec<Value> = post
        .categories
        .iter()
        .map(|c| json!({"name": c.name, "url": links::category_path(site, c)}))
        .collect();
    ctx["heading"] = json!(heading);
    ctx["is_page"] = json!(post.kind == "page");
    ctx["author"] = json!(post.display_author());
    ctx["author_url"] = json!(links::author_path(site, post.author_id));
    ctx["date"] = json!(text::format_php_date(
        site.get_or("postDateFormat", "Y-m-d"),
        post.created,
        site.timezone()
    ));
    ctx["comment_label"] = json!(comment_label);
    ctx["words"] = json!(if view.locked {
        0
    } else {
        text::word_count(&post.text)
    });
    ctx["cats"] = json!(cats);
    ctx["thumb"] = json!(if view.locked {
        Value::Null
    } else {
        json!(text::first_image(&post.text, post.field_str("thumb")))
    });
    ctx["locked"] = json!(view.locked);
    ctx["password_error"] = json!(view.password_error);
    ctx["permalink"] = json!(permalink);
    ctx["cid"] = json!(post.cid);
    ctx["html"] = json!(if view.locked {
        String::new()
    } else if post.kind == "attachment" {
        view.attachment_html.to_string()
    } else {
        text::render_post_html(&post.text)
    });
    ctx["newer_url"] = json!(view.newer.map(|p| links::content_path(site, p)));
    ctx["newer_title"] = json!(view.newer.map(|p| p.title.clone()));
    ctx["older_url"] = json!(view.older.map(|p| links::content_path(site, p)));
    ctx["older_title"] = json!(view.older.map(|p| p.title.clone()));
    ctx["comment_count"] = json!(view.comments.len());
    ctx["comment_html"] = json!(comment_tree_html(chrome, post, view.comments));
    ctx["comment_nav"] = json!(view.comment_nav);
    ctx["ping_html"] = json!(view.ping_html);
    ctx["allow_comment"] = json!(post.allow_comment && !view.locked && comments_open(chrome, post));
    ctx["comment_url"] = json!(links::comment_path(&permalink));
    ctx["comment_error"] = json!(view.comment_error);
    ctx["require_mail"] = json!(site.flag("commentsRequireMail", true));
    ctx["require_url"] = json!(site.flag("commentsRequireURL", false));
    ctx["remember_author"] = json!(view.remember_author);
    ctx["remember_mail"] = json!(view.remember_mail);
    ctx["remember_url"] = json!(view.remember_url);
    ctx["remember_text"] = json!(view.remember_text);
    ctx["reply_to"] = json!(view.reply_to);
    env.get_template("single.html")?.render(ctx)
}

pub fn render_404(env: &Environment<'_>, chrome: &Chrome<'_>) -> Result<String, Error> {
    let base = links::site_base(&chrome.cfg.site_url, chrome.site);
    let static_path = theme_asset_prefix(chrome);
    env.get_template("404.html")?.render(context! {
        site_title => chrome.site.get_or("title", "Tiebidli"),
        description => chrome.site.get("description"),
        year => text::ymd(chrome.now, chrome.site.timezone()).0,
        theme_ready => chrome.theme_ready,
        static => static_path,
        base => base,
    })
}

fn theme_asset_prefix(chrome: &Chrome<'_>) -> String {
    if chrome.theme_ready {
        format!("/usr/themes/{}/assets", chrome.site.theme_name())
    } else {
        String::new()
    }
}

fn shell(
    chrome: &Chrome<'_>,
    page_title: &str,
    description: &str,
    show_tag_cloud: bool,
    post_tags: &[Value],
) -> Value {
    let site = chrome.site;
    let base = links::site_base(&chrome.cfg.site_url, site);
    let (nav_header, nav_collapse, aside_bg) = theme_colors(site);
    let setup = site.theme_list("indexsetup");
    let blog_name = nonempty(&site.theme_str("BlogName"), site.get_or("title", ""));
    let index_name = nonempty(&site.theme_str("IndexName"), site.get_or("title", "首页"));
    let desc = if description.is_empty() {
        site.get("description").to_string()
    } else {
        description.to_string()
    };
    let static_path = theme_asset_prefix(chrome);
    let days = if chrome.stats.first_created > 0 {
        ((chrome.now - chrome.stats.first_created) / 86_400).max(0)
    } else {
        0
    };
    let last = if chrome.stats.last_activity > 0 {
        text::format_php_date(
            site.get_or("postDateFormat", "Y-m-d"),
            chrome.stats.last_activity,
            site.timezone(),
        )
    } else {
        String::new()
    };
    let dark = site.theme_str("dark_setting") == "dark";
    json!({
        "page_title": page_title,
        "site_title": site.get_or("title", "Tiebidli"),
        "description": desc,
        "keywords": site.get("keywords"),
        "base": base,
        "base_json": serde_json::to_string(&base).unwrap_or_else(|_| "\"\"".into()),
        "static": static_path,
        "static_json": serde_json::to_string(&static_path).unwrap_or_else(|_| "\"\"".into()),
        "theme_ready": chrome.theme_ready,
        "layout_class": layout_class(&setup),
        "nav_header": nav_header,
        "nav_collapse": nav_collapse,
        "aside_bg": aside_bg,
        "blog_name": blog_name,
        "blog_job": site.theme_str("BlogJob"),
        "blog_pic": site.theme_str("BlogPic"),
        "index_name": index_name,
        "favicon": site.theme_str("favicon"),
        "year": text::ymd(chrome.now, site.timezone()).0,
        "feed_url": links::feed_path(site, ""),
        "feed_abs": abs(&base, &links::feed_path(site, "")),
        "atom_url": links::feed_path(site, "/atom"),
        "atom_abs": abs(&base, &links::feed_path(site, "/atom")),
        "category_html": category_nav(site),
        "pages": site.pages.iter().filter(|p| p.field_str("navbar") != Some("hide")).map(|p| json!({
            "title": p.title,
            "url": links::content_path(site, p),
        })).collect::<Vec<_>>(),
        "tags": site.tags.iter().filter(|t| t.count > 0).take(30).map(|t| json!({
            "name": t.name,
            "url": links::tag_path(site, t),
        })).collect::<Vec<_>>(),
        "hot": chrome.hot.iter().map(|p| json!({"title": p.title, "url": links::content_path(site, p)})).collect::<Vec<_>>(),
        "random_posts": chrome.random_posts.iter().map(|p| json!({"title": p.title, "url": links::content_path(site, p)})).collect::<Vec<_>>(),
        "recent_comments": chrome.recent.iter().map(|(c, p)| json!({
            "author": c.author,
            "excerpt": text::truncate_chars(&text::plain_text(&c.text), 34),
            "url": format!("{}#comment-{}", links::content_path(site, p), c.coid),
        })).collect::<Vec<_>>(),
        "stat_posts": chrome.stats.posts,
        "stat_comments": chrome.stats.comments,
        "stat_days": days,
        "stat_last": last,
        "bottom_info": site.theme_str("BottomInfo"),
        "bottom_left": site.theme_str("BottomleftInfo"),
        "show_tag_cloud": show_tag_cloud,
        "post_tags": post_tags,
        "dark": dark,
        "dark_json": serde_json::to_string(if dark { "dark" } else { "light" }).unwrap(),
    })
}

fn post_card(chrome: &Chrome<'_>, post: &Post) -> Value {
    let site = chrome.site;
    let locked = post.has_password() && !cookie_unlocks(chrome, post);
    let title = if locked {
        "此内容被密码保护".to_string()
    } else {
        post.title.clone()
    };
    let summary = if locked {
        "请输入密码访问".to_string()
    } else if let Some(custom) = post.field_str("customSummary") {
        custom.to_string()
    } else {
        text::excerpt(&post.text, 140)
    };
    let thumb = if locked {
        None
    } else {
        text::first_image(&post.text, post.field_str("thumb"))
    };
    json!({
        "title": title,
        "url": links::content_path(site, post),
        "summary": summary,
        "thumb": thumb,
        "author": post.display_author(),
        "author_url": links::author_path(site, post.author_id),
        "date": text::format_php_date(site.get_or("postDateFormat", "Y-m-d"), post.created, site.timezone()),
        "comment_label": comment_label(post, locked),
    })
}

fn comment_label(post: &Post, locked: bool) -> String {
    if locked || !post.allow_comment {
        return "关闭评论".into();
    }
    match post.comments_num {
        0 => "暂无评论".into(),
        1 => "1 条评论".into(),
        n => format!("{n} 条评论"),
    }
}

fn comments_open(chrome: &Chrome<'_>, post: &Post) -> bool {
    let site = chrome.site;
    if site.flag("commentsAutoClose", false) {
        let timeout = site.int("commentsPostTimeout", 0);
        if timeout > 0 && chrome.now - post.created > timeout {
            return false;
        }
    }
    true
}

fn cookie_unlocks(chrome: &Chrome<'_>, post: &Post) -> bool {
    let root = crate::auth::site_root(
        &chrome.cfg.site_url,
        &links::site_base(&chrome.cfg.site_url, chrome.site),
    );
    let key = crate::auth::cookie_name(&root, &format!("protectPassword_{}", post.cid));
    chrome
        .cookies
        .get(&key)
        .is_some_and(|v| v == &post.password)
}

fn comment_tree_html(chrome: &Chrome<'_>, post: &Post, comments: &[Comment]) -> String {
    let markdown = chrome.site.flag("commentsMarkdown", false);
    let threaded = chrome.site.flag("commentsThreaded", true);
    let desc = chrome
        .site
        .get("commentsOrder")
        .eq_ignore_ascii_case("DESC");
    let authors: HashMap<i64, String> = comments
        .iter()
        .map(|c| (c.coid, c.author.clone()))
        .collect();
    let mut children: HashMap<i64, Vec<&Comment>> = HashMap::new();
    let mut roots: Vec<&Comment> = Vec::new();
    let ids: HashMap<i64, &Comment> = comments.iter().map(|c| (c.coid, c)).collect();
    for c in comments {
        if threaded && c.parent != 0 && ids.contains_key(&c.parent) {
            children.entry(c.parent).or_default().push(c);
        } else {
            roots.push(c);
        }
    }
    if desc {
        roots.reverse();
        for list in children.values_mut() {
            list.reverse();
        }
    }
    let mut html = String::new();
    for c in roots {
        push_comment(&mut html, chrome, post, c, &children, &authors, markdown, 1);
    }
    html
}

fn push_comment(
    html: &mut String,
    chrome: &Chrome<'_>,
    post: &Post,
    c: &Comment,
    children: &HashMap<i64, Vec<&Comment>>,
    authors: &HashMap<i64, String>,
    markdown: bool,
    depth: usize,
) {
    let class = if depth <= 1 {
        "comment-body comment-parent"
    } else if depth == 2 {
        "comment-body comment-child"
    } else {
        "comment-body comment-child2"
    };
    let mut class = class.to_string();
    if c.author_id != 0 && c.author_id == post.author_id {
        class.push_str(" comment-by-author");
    }
    let author = if c.url.is_empty() || !chrome.site.flag("commentsShowUrl", true) {
        html_escape(&c.author)
    } else {
        let rel = if chrome.site.flag("commentsUrlNofollow", true) {
            " rel=\"external nofollow\""
        } else {
            ""
        };
        format!(
            "<a href=\"{}\" target=\"_blank\"{rel}>{}</a>",
            html_escape(&c.url),
            html_escape(&c.author)
        )
    };
    let avatar = if chrome.site.flag("commentsAvatar", true) && !c.mail.is_empty() {
        let hash = gravatar_hash(&c.mail);
        let prefix = chrome.cfg.gravatar_prefix.trim_end_matches('/');
        let rating = chrome.site.get_or("commentsAvatarRating", "G");
        format!(
            "<img class=\"avatar\" alt=\"\" src=\"{prefix}/{hash}?s=80&amp;r={rating}\" width=\"80\" height=\"80\" loading=\"lazy\">"
        )
    } else {
        String::new()
    };
    let when = text::format_php_date(
        chrome.site.get_or("commentDateFormat", "Y-m-d H:i"),
        c.created,
        chrome.site.timezone(),
    );
    let at = c
        .parent
        .checked_sub(0)
        .and_then(|_| authors.get(&c.parent))
        .map(|name| format!("@{name}"))
        .unwrap_or_default();
    html.push_str(&format!(
        "<li id=\"comment-{id}\" class=\"{class}\"><div id=\"div-comment-{id}\" class=\"comment-body\">\
         <a class=\"pull-left thumb-sm comment-avatar\" rel=\"nofollow\">{avatar}</a>\
         <div class=\"m-b m-l-xxl\"><div class=\"comment-meta\"><span class=\"comment-author vcard\"><b class=\"fn\">{author}</b></span>\
         <div class=\"comment-metadata\"><time class=\"text-muted text-xs block m-t-xs\">{when}</time></div></div>\
         <div class=\"comment-content m-t-sm\"><span class=\"comment-author-at\"><b>{at}</b></span>\
         <div class=\"comment-content-true\">{body}</div></div>\
         <div class=\"comment-reply m-t-sm\"><a href=\"?replyTo={id}#respond\">回复</a></div></div></div>",
        id = c.coid,
        class = class,
        avatar = avatar,
        author = author,
        when = html_escape(&when),
        at = html_escape(&at),
        body = text::render_comment_html(&c.text, markdown),
    ));
    if let Some(list) = children.get(&c.coid) {
        html.push_str("<div class=\"comment-children list-unstyled m-l-xxl\"><ol>");
        for ch in list {
            push_comment(
                html,
                chrome,
                post,
                ch,
                children,
                authors,
                markdown,
                depth + 1,
            );
        }
        html.push_str("</ol></div>");
    }
    html.push_str("</li>");
}

fn gravatar_hash(mail: &str) -> String {
    use md5::{Digest, Md5};
    let normalized = mail.trim().to_lowercase();
    hex::encode(Md5::digest(normalized.as_bytes()))
}

fn category_nav(site: &Site) -> String {
    fn walk(site: &Site, parent: i64) -> String {
        let mut html = String::new();
        for c in site.categories.iter().filter(|c| c.parent == parent) {
            html.push_str(&format!(
                "<li><a href=\"{}\"><span>{}</span></a>",
                html_escape(&links::category_path(site, c)),
                html_escape(&c.name)
            ));
            let child = walk(site, c.mid);
            if !child.is_empty() {
                html.push_str("<ul class=\"nav nav-sub dk child-nav\">");
                html.push_str(&child);
                html.push_str("</ul>");
            }
            html.push_str("</li>");
        }
        html
    }
    walk(site, 0)
}

fn theme_colors(site: &Site) -> (&'static str, &'static str, &'static str) {
    let raw = site.theme_str("themetype");
    let n: usize = raw.parse().unwrap_or(1);
    let idx = n.saturating_sub(1).min(PRESETS.len() - 1);
    PRESETS[idx]
}

fn layout_class(setup: &[String]) -> String {
    let defaults = ["header-fix".to_string(), "aside-fix".to_string()];
    let setup: Vec<&str> = if setup.is_empty() {
        defaults.iter().map(|s| s.as_str()).collect()
    } else {
        setup.iter().map(|s| s.as_str()).collect()
    };
    let mut parts = vec!["app".to_string()];
    if setup.contains(&"aside-fix") {
        parts.push("app-aside-fix".into());
    }
    if setup.contains(&"aside-folded") {
        parts.push("app-aside-folded".into());
    }
    if setup.contains(&"aside-dock") {
        parts.push("app-aside-dock".into());
    }
    if setup.contains(&"container-box") {
        parts.push("container".into());
    } else {
        parts.push("no-container".into());
    }
    if setup.contains(&"header-fix") {
        parts.push("app-header-fixed".into());
    }
    parts.join(" ")
}

fn format_title(heading: &str, chrome: &Chrome<'_>, page: i64) -> String {
    let site_title = chrome.site.get_or("title", "Tiebidli");
    let prefix = if page > 1 {
        format!("第{page}页 - ")
    } else {
        String::new()
    };
    if heading == site_title {
        format!("{prefix}{site_title}")
    } else {
        format!("{prefix}{heading} - {site_title}")
    }
}

fn nonempty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

#[allow(dead_code)]
fn _meta(m: &Meta) -> &str {
    &m.name
}
