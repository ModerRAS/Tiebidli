# Tiebidli

用 Rust 和 Axum 写的 Typecho 1.2 博客引擎，直接读取已经装好的数据库。不改表结构。公开站、XML-RPC 和最小后台都在这一版里。

表结构以 Typecho `v1.2.1` 的 `install/Mysql.sql`、`install/SQLite.sql` 为准，副本在 `schema/`。核心表没有 `views` 列；插件加出来的列不会被查询，也不会被改掉。

## 环境变量

复制 `.env.example` 为 `.env`。程序只在变量尚未设置时读取 `.env`。

| 变量 | 作用 |
| --- | --- |
| `DATABASE_URL` | `mysql://user:pass@127.0.0.1:3306/typecho` 或 `sqlite:///绝对路径/typecho.db` |
| `TYPECHO_PREFIX` | 表前缀，默认 `typecho_` |
| `SITE_URL` | 站点根地址，用来生成 RSS 绝对链接和 cookie 前缀。留空则用库里的 `siteUrl` |
| `LISTEN` | 监听地址，默认 `0.0.0.0:8080` |
| `THEME_STATIC_DIR` | 主题静态文件目录，挂到 `/usr/themes/{theme}/`，默认 `./themes/default` |
| `THEME_DIR` | 可选。同名 html 覆盖内置模板 |
| `UPLOADS_DIR` | 上传目录，映射到 `/usr/uploads/` |
| `GRAVATAR_PREFIX` | 评论头像前缀，默认 Gravatar |
| `STATIC_DIR` | 自带样式目录，默认 `./static` |

PostgreSQL 的安装 SQL 只作对照，这一版不会连接它。

## 运行

需要 Rust stable（本仓库在 1.99 上通过 `cargo test`；sqlx 0.8 需要较新的编译器）。

```bash
cargo run --release
```

开发检查：

```bash
cargo test
```

后台入口是 `/admin/index.php`。未登录会转到 `/admin/login.php?referer=`。注册取决于 `allowRegister`，默认关闭。页面文件名与 Typecho 1.2.1 相同，例如 `/admin/write-post.php`、`/admin/manage-posts.php`、`/admin/profile.php`、`/admin/options-permalink.php`。表单提交走 `/action/...`，参数名是 `do`，并带 `_` 安全 token；登录固定走 `/index.php/action/login`。顶栏用户名进入个人设置。评论列表可以行内编辑和回复。撰写页可以预览当前未保存的内容，预览走公开站的 `Renderer`。自定义字段写入 `typecho_fields`。引用通告文本框会解析地址，远程 ping 这一版不发。分类有独立编辑页，也可以把选中分类合并到另一个分类。文件可以单条编辑；撰写页的附件页列出当前文章的文件，并能上传、插入和删除。


## 主题

公开页通过 `Renderer` 输出 HTML。内置模板是默认实现。把一套主题的静态文件放到 `THEME_STATIC_DIR`，并把 `options.theme` 设成目录名，文件会出现在 `/usr/themes/{theme}/`。默认模板在目录里存在 `assets/css/style.css` 时引用它，否则用 `/static/themes/default/` 里的 Typecho 1.2.1 默认主题样式。

要用自己的 HTML，把 `base.html`、`list.html`、`single.html`、`search.html`、`404.html` 放到 `THEME_DIR`。进程启动时同名文件覆盖内置模板。换模板引擎时实现同一个 `Renderer` 即可，HTTP 层不绑定具体引擎。

主题文件不要放进 git。

## 路由

默认规则与 Typecho 1.2.1 `install.php` 里的 `install_get_default_routers()` 相同，例如：

- `/`、`/page/2/`
- `/archives/{cid}/`、`/attachment/{cid}/`
- `/category/{slug}/`、`/tag/{slug}/`
- `/author/{uid}/`
- `/search/{keywords}/`，搜索框的 `?s=` 会跳到这里
- `/2024/01/14/`
- `/{slug}.html`
- `/feed/`（RSS 2.0）、`/feed/rss/`（RSS 1.0）、`/feed/atom/`、`/feed/comments/`
- `/action/xmlrpc`

`options.rewrite` 为 `0` 时，生成的前台链接带 `/index.php` 前缀（例如 `/index.php/archives/1/`）。为 `1` 时输出干净路径。两种入口都能打开同一页。和原版 `Archive::checkPermalink` 一样，请求路径和永久链接不一致时 301；首页、评论分页、评论提交和 404 不跳。搜索框的 `?s=` 是 302。

永久链接页提供默认风格、wordpress 风格、按日期归档、按分类归档和个性化定义，也可以改独立页面路径和分类路径。占位符语义与 `Router\Parser` 相同，例如 `[cid:digital]`、`[slug]`、`[directory:split:0]`、`[year:digital:4]`。

库里的 `routingTable` 会按同名规则覆盖 `url`，匹配顺序仍是 1.2 的默认顺序。另外接受 `/archives/{slug}` 和 `/archives/{slug}.html`，然后 301 到当前永久链接。

## 和 1.2 对齐的行为

- 列表只显示 `status=publish` 且 `created` 早于当前时间的文章。`hidden` 可以用直链打开。`private`、`draft`、`waiting` 对匿名访客不可见。
- 密码文章留在列表里。cookie 名是 `md5(rootUrl)` 接上 `protectPassword_{cid}`，值和明文密码一致才显示正文。
- 正文以 `<!--markdown-->` 开头时按 Markdown 渲染，否则按空行分段。`<!--more-->` 只影响列表摘要。
- 评论支持审核、白名单、停用词、IP 黑名单、发言间隔、分页和引用。`commentsPageBreak` 关闭时一次输出全部顶层评论。
- 静态首页读 `frontPage=page:{cid}`。`frontPage` 不是 `recent` 且 `frontArchive` 打开时，`/blog/` 才是文章列表。

## 这一版不做

- 主题市场、插件安装和插件钩子
- 备份导入导出、安装向导、版本升级迁移
- 短代码、PJAX、播放器、AMP
- `frontPage=file:` 指定的主题文件
- 反垃圾 JS token
- 热门文章按 `commentsNum`，因为 1.2 核心没有 `views`
- 日期归档用站点时区的 `[开始, 次日)`，和 Typecho 含当天 23:59:59 的写法差一秒

更细的请求路径见 `docs/architecture.md`，字段见 `docs/typecho-1.2-schema.md`。
