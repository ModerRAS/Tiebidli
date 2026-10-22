use crate::model::{Meta, Post, Site};
use crate::text::{self, html_escape};
use std::collections::HashMap;

pub fn enc(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

pub fn site_base(cfg_site: &Option<String>, site: &Site) -> String {
    if let Some(u) = cfg_site {
        let u = u.trim();
        if !u.is_empty() {
            return u.trim_end_matches('/').to_string();
        }
    }
    let o = site.get("siteUrl").trim();
    if !o.is_empty() {
        return o.trim_end_matches('/').to_string();
    }
    "http://127.0.0.1:8080".into()
}

pub fn abs(base: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    format!("{base}{path}")
}

pub fn post_values(site: &Site, post: &Post) -> HashMap<String, String> {
    let tz = site.timezone();
    let (y, m, d) = text::ymd(post.created, tz);
    let mut vals = HashMap::new();
    vals.insert("cid".into(), post.cid.to_string());
    vals.insert("slug".into(), enc(&post.slug));
    vals.insert("year".into(), format!("{y:04}"));
    vals.insert("month".into(), format!("{m:02}"));
    vals.insert("day".into(), format!("{d:02}"));
    let (category, directory_raw, directory_enc) = directory_of(site, post);
    vals.insert("category".into(), enc(&category));
    vals.insert("directory".into(), directory_enc);
    vals.insert("directory_raw".into(), directory_raw);
    vals.insert("uid".into(), post.author_id.to_string());
    vals
}

pub fn content_path(site: &Site, post: &Post) -> String {
    let name = match post.kind.as_str() {
        "page" => "page",
        "attachment" => "attachment",
        _ => "post",
    };
    let vals = post_values(site, post);
    site.routes
        .url(name, &vals)
        .unwrap_or_else(|| format!("/archives/{}/", post.cid))
}

pub fn author_path(site: &Site, uid: i64) -> String {
    let mut vals = HashMap::new();
    vals.insert("uid".into(), uid.to_string());
    site.routes
        .url("author", &vals)
        .unwrap_or_else(|| format!("/author/{uid}/"))
}

pub fn category_vals(site: &Site, meta: &Meta) -> HashMap<String, String> {
    let mut vals = HashMap::new();
    vals.insert("slug".into(), enc(&meta.slug));
    vals.insert("mid".into(), meta.mid.to_string());
    let (raw, encoded) = directory_of_mid(site, meta.mid);
    vals.insert("directory".into(), encoded);
    vals.insert("directory_raw".into(), raw);
    vals.insert("category".into(), enc(&meta.slug));
    vals
}

pub fn category_path(site: &Site, meta: &Meta) -> String {
    let vals = category_vals(site, meta);
    site.routes
        .url("category", &vals)
        .unwrap_or_else(|| format!("/category/{}/", enc(&meta.slug)))
}

pub fn tag_path(site: &Site, meta: &Meta) -> String {
    let mut vals = HashMap::new();
    vals.insert("slug".into(), enc(&meta.slug));
    site.routes
        .url("tag", &vals)
        .unwrap_or_else(|| format!("/tag/{}/", enc(&meta.slug)))
}

pub fn paged_path(
    site: &Site,
    base: &str,
    paged: &str,
    mut vals: HashMap<String, String>,
    page: i64,
) -> String {
    if page <= 1 {
        site.routes.url(base, &vals).unwrap_or_else(|| "/".into())
    } else {
        vals.insert("page".into(), page.to_string());
        site.routes
            .url(paged, &vals)
            .unwrap_or_else(|| format!("/page/{page}/"))
    }
}

pub fn feed_path(site: &Site, suffix: &str) -> String {
    let mut vals = HashMap::new();
    vals.insert("feed".into(), suffix.to_string());
    site.routes
        .url("feed", &vals)
        .unwrap_or_else(|| format!("/feed{suffix}"))
}

pub fn comment_page_path(site: &Site, post: &Post, page: i64) -> String {
    let permalink = content_path(site, post);
    if page <= 1 {
        return permalink;
    }
    let bare = permalink.trim_matches('/');
    let mut vals = HashMap::new();
    vals.insert("permalink".into(), bare.to_string());
    vals.insert("commentPage".into(), page.to_string());
    site.routes
        .url("comment_page", &vals)
        .unwrap_or_else(|| format!("/{bare}/comment-page-{page}"))
}

pub fn comment_path(permalink: &str) -> String {
    if permalink.ends_with('/') {
        format!("{permalink}comment")
    } else {
        format!("{permalink}/comment")
    }
}

fn directory_of(site: &Site, post: &Post) -> (String, String, String) {
    let Some(first) = post.categories.first() else {
        return (String::new(), String::new(), String::new());
    };
    let (raw, encoded) = directory_of_mid(site, first.mid);
    (first.slug.clone(), raw, encoded)
}

fn directory_of_mid(site: &Site, mid: i64) -> (String, String) {
    let mut chain = Vec::new();
    let mut current = mid;
    for _ in 0..24 {
        let Some(m) = site.categories.iter().find(|c| c.mid == current) else {
            break;
        };
        chain.push(m.slug.clone());
        if m.parent == 0 || m.parent == current {
            break;
        }
        current = m.parent;
    }
    chain.reverse();
    let raw = chain.join("/");
    let encoded = chain.iter().map(|s| enc(s)).collect::<Vec<_>>().join("/");
    (raw, encoded)
}

pub fn child_mids(all: &[Meta], root: i64) -> Vec<i64> {
    let mut out = vec![root];
    let mut stack = vec![root];
    let mut guard = 0;
    while let Some(mid) = stack.pop() {
        guard += 1;
        if guard > 200 {
            break;
        }
        for m in all {
            if m.parent == mid && !out.contains(&m.mid) {
                out.push(m.mid);
                stack.push(m.mid);
            }
        }
    }
    out
}

pub fn pagination(
    current: i64,
    total: i64,
    page_size: i64,
    href_for: impl Fn(i64) -> String,
) -> String {
    if total <= page_size || page_size <= 0 {
        return String::new();
    }
    let pages = (total + page_size - 1) / page_size;
    let current = current.clamp(1, pages);
    let mut html = String::from("<ol class=\"page-navigator\">");
    if current > 1 {
        html.push_str(&format!(
            "<li class=\"prev\"><a href=\"{}\"><i class=\"fontello fontello-chevron-left\"></i></a></li>",
            html_escape(&href_for(current - 1))
        ));
    }
    let mut window = Vec::new();
    window.push(1);
    for n in (current - 2).max(1)..=(current + 2).min(pages) {
        window.push(n);
    }
    window.push(pages);
    window.sort_unstable();
    window.dedup();
    let mut prev = 0;
    for n in window {
        if prev != 0 && n > prev + 1 {
            html.push_str("<li><span>...</span></li>");
        }
        if n == current {
            html.push_str(&format!("<li class=\"current\"><span>{n}</span></li>"));
        } else {
            html.push_str(&format!(
                "<li><a href=\"{}\">{n}</a></li>",
                html_escape(&href_for(n))
            ));
        }
        prev = n;
    }
    if current < pages {
        html.push_str(&format!(
            "<li class=\"next\"><a href=\"{}\"><i class=\"fontello fontello-chevron-right\"></i></a></li>",
            html_escape(&href_for(current + 1))
        ));
    }
    html.push_str("</ol>");
    html
}
