//! 后台「插件」页上的功能分组。
//!
//! 这里的插件就是普通的进程内功能：和站点其它代码编进同一个二进制，
//! 启动时放进一份静态登记表。启用、停用只改 options，并决定程序里
//! 已经写好的钩子会不会跑。边界是组织形式（trait、登记表、options），
//! 用来做和 Typecho 一样的管理入口。
//!
//! 没有动态加载。不加载 DLL，不 `dlopen` `.so`，不在运行时载入插件 crate，也不跑 WASM。
//! 功能仍写在原来的 Rust 模块里；只有需要出现在插件管理页时才登记到这里。
//! 本轮登记表是空的，页面保持「还没有安装插件」。

use crate::model::Site;

pub trait Feature: Send + Sync {
    /// 选项和钩子使用的短名，对应 Typecho 的插件目录名。
    fn name(&self) -> &'static str;
    fn title(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// `options` 里的开关。`"0"` 关闭，其它已有值视为开启。缺省关闭。
    fn option_key(&self) -> &'static str;
}

pub fn registry() -> &'static [&'static dyn Feature] {
    &[]
}

pub fn is_enabled(site: &Site, feature: &dyn Feature) -> bool {
    match site.get(feature.option_key()) {
        "" | "0" => false,
        _ => true,
    }
}

pub fn page_note(site: &Site) -> String {
    let mut rows = String::new();
    for feature in registry() {
        let state = if is_enabled(site, *feature) {
            "启用"
        } else {
            "禁用"
        };
        rows.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            crate::text::html_escape(feature.title()),
            crate::text::html_escape(feature.name()),
            crate::text::html_escape(feature.description()),
            state
        ));
    }
    let table = if rows.is_empty() {
        "<div class=\"message notice\"><p>还没有安装插件。</p></div>".to_string()
    } else {
        format!(
            "<div class=\"typecho-table-wrap\"><table class=\"typecho-list-table\"><thead><th>名称</th><th>标识</th><th>描述</th><th>状态</th></thead><tbody>{rows}</tbody></table></div>"
        )
    };
    format!(
        r#"<div class="row typecho-page-main"><div class="col-mb-12">{table}<p class="description">这里列出的是编译进同一程序的功能，用来启用、停用和配置。启用只改选项，并决定程序里已经写好的钩子会不会运行。</p></div></div>"#
    )
}
