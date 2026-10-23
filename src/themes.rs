//! 外观目录发现和默认外观的设置。
//!
//! `THEME_STATIC_DIR` 指向 `themes/{name}` 里的一个外观目录。它的父目录是外观根，
//! 和 Typecho 的 `usr/themes` 一样，每个子目录是一套外观。启用只写 `options.theme`。
//! 公开页的 HTML 仍由内置模板或 `THEME_DIR` 输出，这里不执行 PHP，也不解析 `functions.php`。
//!
//! 默认外观（目录名 `default`）有一份内置设置：`logoUrl` 和 `sidebarBlock`。
//! 其它目录可以启用，没有单独的设置表单。

use crate::config::Config;
use crate::db::Db;
use crate::model::{theme_directory_name, Site};
use crate::phpval::{self, PhpValue};
use crate::text::html_escape;
use std::path::{Path, PathBuf};

pub const SIDEBAR_BLOCKS: &[(&str, &str)] = &[
    ("ShowRecentPosts", "显示最新文章"),
    ("ShowRecentComments", "显示最近回复"),
    ("ShowCategory", "显示分类"),
    ("ShowArchive", "显示归档"),
    ("ShowOther", "显示其它杂项"),
];

const SCREEN_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "webp", "avif"];

pub fn theme_name_ok(name: &str) -> bool {
    !name.is_empty() && theme_directory_name(name) == name
}

pub fn themes_root(theme_dir: &Path) -> PathBuf {
    theme_dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn parent_is_temp(path: &Path) -> bool {
    path == std::env::temp_dir()
}

/// 某个外观的静态文件目录。
/// 外观根不是系统临时目录时，优先用 `{根}/{name}`。
/// 否则沿用 `THEME_STATIC_DIR`，这样单独挂一个目录时公开页仍然读得到样式。
pub fn locate(cfg: &Config, requested: &str, active: &str) -> PathBuf {
    let name = if theme_name_ok(requested) {
        requested.to_string()
    } else {
        "default".into()
    };
    let root = themes_root(&cfg.theme_dir);
    let child = root.join(&name);
    if !parent_is_temp(&root) && child.is_dir() {
        return child;
    }
    if requested == active || name == active {
        return cfg.theme_dir.clone();
    }
    child
}

pub fn style_ready(cfg: &Config, theme_name: &str) -> bool {
    locate(cfg, theme_name, theme_name)
        .join("assets/css/style.css")
        .is_file()
}

pub fn has_config(name: &str) -> bool {
    name == "default"
}

pub fn installed(cfg: &Config, name: &str) -> bool {
    if !theme_name_ok(name) {
        return false;
    }
    let root = themes_root(&cfg.theme_dir);
    if !parent_is_temp(&root) && root.join(name).is_dir() {
        return true;
    }
    let base = cfg
        .theme_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    cfg.theme_dir.is_dir() && base == name
}

pub struct ThemeRow {
    pub name: String,
    pub title: String,
    pub author: String,
    pub homepage: String,
    pub version: String,
    pub description: String,
    pub screen: String,
    pub current: bool,
}

pub fn discover(cfg: &Config, active: &str, missing: bool) -> Vec<ThemeRow> {
    let mut names = Vec::new();
    let root = themes_root(&cfg.theme_dir);
    if !parent_is_temp(&root) {
        if let Ok(rd) = std::fs::read_dir(&root) {
            for entry in rd.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let Some(name) = entry.file_name().to_str().map(|s| s.to_string()) else {
                    continue;
                };
                if theme_name_ok(&name) {
                    names.push(name);
                }
            }
        }
    }
    if names.is_empty() {
        if let Some(name) = cfg.theme_dir.file_name().and_then(|s| s.to_str()) {
            if theme_name_ok(name) && cfg.theme_dir.is_dir() {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    if let Some(pos) = names.iter().position(|n| n == active) {
        let current = names.remove(pos);
        names.insert(0, current);
    }
    names
        .into_iter()
        .map(|name| {
            let dir = if !parent_is_temp(&themes_root(&cfg.theme_dir)) {
                themes_root(&cfg.theme_dir).join(&name)
            } else {
                cfg.theme_dir.clone()
            };
            let (title, author, homepage, version, description) = meta(&name);
            ThemeRow {
                current: name == active && !missing,
                screen: screen_url(&dir, &name),
                name,
                title,
                author,
                homepage,
                version,
                description,
            }
        })
        .collect()
}

fn meta(name: &str) -> (String, String, String, String, String) {
    if name == "default" {
        (
            "默认外观".into(),
            "Typecho Team".into(),
            "http://typecho.org".into(),
            "1.2".into(),
            "内置默认外观。可以设置站点 LOGO 和侧栏里显示的模块。".into(),
        )
    } else {
        (
            name.into(),
            String::new(),
            String::new(),
            String::new(),
            "这个目录提供静态文件，页面仍由内置模板或 THEME_DIR 输出。".into(),
        )
    }
}

fn screen_url(dir: &Path, name: &str) -> String {
    if let Some(file) = screenshot_file(dir) {
        format!("/usr/themes/{name}/{file}")
    } else {
        "/static/admin/img/noscreen.png".into()
    }
}

fn screenshot_file(dir: &Path) -> Option<String> {
    let rd = std::fs::read_dir(dir).ok()?;
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let lower = file_name.to_ascii_lowercase();
        let Some(stem) = lower.rsplit_once('.') else {
            continue;
        };
        if stem.0 == "screenshot" && SCREEN_EXTS.contains(&stem.1) {
            return Some(file_name.to_string());
        }
    }
    None
}

pub fn tabs(current: &str, show_config: bool) -> String {
    let item = |key: &str, href: &str, label: &str| {
        let class = if key == current {
            " class=\"current\""
        } else {
            ""
        };
        format!("<li{class}><a href=\"{href}\">{label}</a></li>")
    };
    let mut html = String::from("<ul class=\"typecho-option-tabs fix-tabs clearfix\">");
    html.push_str(&item("themes", "/admin/themes.php", "可以使用的外观"));
    html.push_str(&item("editor", "/admin/theme-editor.php", "编辑当前外观"));
    if show_config {
        html.push_str(&item("config", "/admin/options-theme.php", "设置外观"));
    }
    html.push_str("</ul>");
    html
}

pub fn themes_main(cfg: &Config, site: &Site, flash: &str) -> String {
    let active = site.theme_name();
    let stored = site.get("theme");
    let missing = if stored.is_empty() {
        !installed(cfg, &active)
    } else {
        !installed(cfg, stored)
    };
    let missing_name = if stored.is_empty() {
        active.clone()
    } else {
        stored.to_string()
    };
    let rows = discover(cfg, &active, missing);
    let mut body = String::new();
    if missing {
        body.push_str(&format!(
            r#"<tr id="theme-{name}" class="current"><td colspan="2" class="warning"><p><strong>检测到您之前使用的 "{name}" 外观文件不存在，您可以重新上传此外观或者启用其他外观。</strong></p><ul><li>重新上传此外观后刷新当前页面，此提示将会消失。</li><li>启用新外观后，当前外观的设置数据将被删除。</li></ul></td></tr>"#,
            name = html_escape(&missing_name),
        ));
    }
    for row in &rows {
        let class = if row.current { " class=\"current\"" } else { "" };
        let title = if row.title.is_empty() {
            row.name.clone()
        } else {
            row.title.clone()
        };
        let mut cite = String::new();
        if !row.author.is_empty() {
            let author = if row.homepage.is_empty() {
                html_escape(&row.author)
            } else {
                format!(
                    "<a href=\"{}\">{}</a>",
                    html_escape(&row.homepage),
                    html_escape(&row.author)
                )
            };
            cite.push_str(&format!("作者: {author} &nbsp;&nbsp;"));
        }
        if !row.version.is_empty() {
            cite.push_str(&format!("版本: {}", html_escape(&row.version)));
        }
        let mut actions = String::new();
        if active != row.name || missing {
            actions = format!(
                r#"<p><a class="edit" href="/admin/theme-editor.php?theme={name}">编辑</a> &nbsp; <a class="activate" href="/action/themes-edit?change={name}">启用</a></p>"#,
                name = html_escape(&row.name),
            );
        }
        body.push_str(&format!(
            r#"<tr id="theme-{name}"{class}><td valign="top"><img src="{screen}" alt="{name}"></td><td valign="top"><h3>{title}</h3><cite>{cite}</cite><p>{desc}</p>{actions}</td></tr>"#,
            name = html_escape(&row.name),
            class = class,
            screen = html_escape(&row.screen),
            title = html_escape(&title),
            desc = nl2br(&row.description),
        ));
    }
    format!(
        r#"<div class="row typecho-page-main" role="main"><div class="col-mb-12">{flash}{tabs}<div class="typecho-table-wrap"><table class="typecho-list-table typecho-theme-list"><colgroup><col width="35%"><col></colgroup><thead><th>截图</th><th>详情</th></thead><tbody>{body}</tbody></table></div><p class="description">静态文件放在外观目录。模板 HTML 由 THEME_DIR 在启动时覆盖同名文件。</p></div></div>"#,
        tabs = tabs("themes", has_config(&active) && !missing),
    )
}

pub fn editor_main(cfg: &Config, site: &Site, flash: &str) -> String {
    let active = site.theme_name();
    let mut files = String::new();
    if let Some(dir) = &cfg.template_dir {
        if let Ok(rd) = std::fs::read_dir(dir) {
            let mut names: Vec<String> = rd
                .flatten()
                .filter_map(|entry| {
                    let name = entry.file_name().to_str()?.to_string();
                    if name.ends_with(".html") && entry.path().is_file() {
                        Some(name)
                    } else {
                        None
                    }
                })
                .collect();
            names.sort();
            if !names.is_empty() {
                files.push_str("<ul>");
                for name in names {
                    files.push_str(&format!("<li>{}</li>", html_escape(&name)));
                }
                files.push_str("</ul>");
            }
        }
    }
    let where_dir = cfg
        .template_dir
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "未设置".into());
    format!(
        r#"<div class="row typecho-page-main" role="main"><div class="col-mb-12">{flash}{tabs}<div class="message notice"><p>当前外观「{active}」的页面由内置模板输出。把 base.html、list.html、single.html、search.html、404.html 放到 THEME_DIR 后，下次启动会覆盖同名模板。</p><p>这里不编辑 PHP 外观，也不执行 functions.php。</p><p>THEME_DIR：{dir}</p>{files}</div></div></div>"#,
        tabs = tabs("editor", has_config(&active)),
        active = html_escape(&active),
        dir = html_escape(&where_dir),
    )
}

pub fn config_main(site: &Site, flash: &str) -> Result<String, &'static str> {
    let active = site.theme_name();
    if !has_config(&active) {
        return Err("外观配置功能不存在");
    }
    let logo = site.theme_str("logoUrl");
    let selected = match site.theme.get_key("sidebarBlock") {
        Some(value) => value.string_list(),
        None => Vec::new(),
    };
    let mut boxes = String::new();
    for (value, label) in SIDEBAR_BLOCKS {
        let checked = if selected.iter().any(|item| item == value) {
            " checked=\"checked\""
        } else {
            ""
        };
        boxes.push_str(&format!(
            r#"<span class="multiline"><input name="sidebarBlock[]" type="checkbox" value="{value}" id="sidebarBlock-{value}"{checked}><label for="sidebarBlock-{value}">{label}</label></span>"#,
            label = html_escape(label),
        ));
    }
    Ok(format!(
        r#"<div class="row typecho-page-main" role="main"><div class="col-mb-12">{tabs}</div><div class="col-mb-12 col-tb-8 col-tb-offset-2" role="form">{flash}<form action="/action/themes-edit?config" method="post"><ul class="typecho-option"><li><label class="typecho-label" for="logoUrl">站点 LOGO 地址</label><input id="logoUrl" name="logoUrl" type="text" class="w-100" value="{logo}"><p class="description">在这里填入一个图片 URL 地址, 以在网站标题前加上一个 LOGO</p></li></ul><ul class="typecho-option"><li><label class="typecho-label">侧边栏显示</label>{boxes}</li></ul><ul class="typecho-option typecho-option-submit"><li><button type="submit" class="btn primary">保存设置</button></li></ul></form></div></div>"#,
        tabs = tabs("config", true),
        logo = html_escape(&logo),
    ))
}

pub fn default_theme_value() -> String {
    theme_config_value("", &SIDEBAR_BLOCKS.iter().map(|(k, _)| *k).collect::<Vec<_>>())
}

pub fn theme_config_value(logo: &str, blocks: &[&str]) -> String {
    let items = blocks
        .iter()
        .enumerate()
        .map(|(i, value)| (PhpValue::Int(i as i64), PhpValue::String((*value).into())))
        .collect();
    phpval::serialize(&PhpValue::Array(vec![
        (
            PhpValue::String("logoUrl".into()),
            PhpValue::String(logo.into()),
        ),
        (PhpValue::String("sidebarBlock".into()), PhpValue::Array(items)),
    ]))
}

pub async fn change_theme(db: &Db, cfg: &Config, site: &Site, requested: &str) -> Result<(), &'static str> {
    let name = requested.trim_matches(|c| c == '.' || c == '/').trim();
    if !installed(cfg, name) {
        return Err("您选择的风格不存在");
    }
    let effective = site.theme_name();
    let stored = site.get("theme");
    let old = if stored.is_empty() {
        effective.clone()
    } else {
        stored.to_string()
    };
    if name != old {
        db.delete_option(&format!("theme:{old}"))
            .await
            .map_err(|_| "外观设置保存失败")?;
        if site.get("frontPage").starts_with("file:") {
            db.set_option("frontPage", "recent")
                .await
                .map_err(|_| "外观设置保存失败")?;
        }
        if has_config(name) {
            db.set_option(&format!("theme:{name}"), &default_theme_value())
                .await
                .map_err(|_| "外观设置保存失败")?;
        }
    }
    db.set_option("theme", name)
        .await
        .map_err(|_| "外观设置保存失败")?;
    Ok(())
}

pub async fn save_theme_config(db: &Db, site: &Site, logo: &str, posted: &[String]) -> Result<(), &'static str> {
    let name = site.theme_name();
    if !has_config(&name) {
        return Err("外观配置功能不存在");
    }
    let mut logo = logo.trim().to_string();
    if logo.len() > 2000 {
        logo.truncate(2000);
    }
    let blocks: Vec<&str> = SIDEBAR_BLOCKS
        .iter()
        .map(|(key, _)| *key)
        .filter(|key| posted.iter().any(|item| item == key))
        .collect();
    let value = theme_config_value(&logo, &blocks);
    db.set_option(&format!("theme:{name}"), &value)
        .await
        .map_err(|_| "外观设置保存失败")?;
    Ok(())
}

fn nl2br(s: &str) -> String {
    html_escape(s).replace('\n', "<br>\n")
}
