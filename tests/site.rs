use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tiebidli::db::Db;
use tiebidli::{build_router, AppState, Config};
use tower::ServiceExt;

static N: AtomicU64 = AtomicU64::new(0);

async fn boot() -> (axum::Router, Db, std::path::PathBuf) {
    let n = N.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "tiebidli-{}-{}-{n}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&path);
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let mut cfg = Config::for_test(&url);
    let uploads = path.with_extension("uploads");
    std::fs::create_dir_all(&uploads).expect("uploads");
    cfg.uploads_dir = Some(uploads.clone());
    let marker = cfg.theme_dir.join("assets/css/style.css");
    std::fs::create_dir_all(marker.parent().unwrap()).expect("theme marker dir");
    std::fs::write(&marker, "/* test */").expect("theme marker");
    let db = Db::connect(&url, &cfg.prefix).await.expect("connect");
    for stmt in include_str!("../schema/typecho-1.2-sqlite.sql").split(';') {
        let stmt = stmt.trim();
        if stmt.is_empty() {
            continue;
        }
        sqlx::query(stmt)
            .execute(&db.pool)
            .await
            .unwrap_or_else(|e| panic!("schema: {e}\n{stmt}"));
    }
    seed(&db).await;
    let app = build_router(Arc::new(AppState::new(db.clone(), cfg)));
    (app, db, uploads)
}

async fn seed(db: &Db) {
    sqlx::query(
        r#"INSERT INTO "typecho_users"
        ("uid","name","password","mail","url","screenName","created","activated","logged","group","authCode")
        VALUES (1, 'admin', 'x', 'admin@example.com', 'https://example.com', '站长', 1600000000, 1, 0, 'administrator', NULL)"#,
    )
    .execute(&db.pool)
    .await
    .unwrap();

    let posts = [
        (
            1_i64,
            "你好世界",
            "hello-world",
            1_700_000_000_i64,
            "<!--markdown-->第一段\n\n<!--more-->\n\n秘密段落",
            "publish",
            "",
            1_i64,
            "post",
        ),
        (
            2,
            "第二篇",
            "second-post",
            1_700_001_000,
            "<!--markdown-->第二篇正文",
            "publish",
            "",
            0,
            "post",
        ),
        (
            3,
            "密码文",
            "secret-post",
            1_690_000_000,
            "<!--markdown-->密码正文",
            "publish",
            "s3cret",
            0,
            "post",
        ),
        (
            4,
            "隐藏文",
            "hidden-post",
            1_700_000_500,
            "<!--markdown-->隐藏正文",
            "hidden",
            "",
            0,
            "post",
        ),
        (
            5,
            "未来文",
            "future-post",
            4_000_000_000,
            "<!--markdown-->还没发布",
            "publish",
            "",
            0,
            "post",
        ),
        (
            6,
            "关于",
            "about",
            1_700_000_000,
            "<!--markdown-->关于本站",
            "publish",
            "",
            0,
            "page",
        ),
    ];
    for (cid, title, slug, created, text, status, password, comments, kind) in posts {
        let allow_feed = if cid == 3 { "0" } else { "1" };
        sqlx::query(
            r#"INSERT INTO "typecho_contents"
            ("cid","title","slug","created","modified","text","order","authorId","template","type","status","password","commentsNum","allowComment","allowPing","allowFeed","parent")
            VALUES (?, ?, ?, ?, ?, ?, 0, 1, NULL, ?, ?, ?, ?, '1', '1', ?, 0)"#,
        )
        .bind(cid)
        .bind(title)
        .bind(slug)
        .bind(created)
        .bind(created)
        .bind(text)
        .bind(kind)
        .bind(status)
        .bind(password)
        .bind(comments)
        .bind(allow_feed)
        .execute(&db.pool)
        .await
        .unwrap();
    }

    sqlx::query(
        r#"INSERT INTO "typecho_metas"
        ("mid","name","slug","type","description","count","order","parent")
        VALUES (1, '随笔', 'notes', 'category', '随笔分类', 2, 0, 0),
               (2, 'Rust', 'rust', 'tag', '', 1, 0, 0)"#,
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO "typecho_relationships" ("cid","mid") VALUES (1, 1), (1, 2), (2, 1)"#,
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO "typecho_fields" ("cid","name","type","str_value","int_value","float_value")
           VALUES (2, 'customSummary', 'str', '自定义摘要', 0, 0)"#,
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO "typecho_comments"
        ("cid","created","author","authorId","ownerId","mail","url","ip","agent","text","type","status","parent")
        VALUES (1, 1700000100, '旧读者', 0, 1, 'old@example.com', NULL, '127.0.0.1', 'test', '早已在此', 'comment', 'approved', 0)"#,
    )
    .execute(&db.pool)
    .await
    .unwrap();

    let theme = r#"a:2:{s:8:"BlogName";s:6:"测试";s:7:"BlogJob";s:12:"写点东西";}"#;
    let options = [
        ("title", "测试博客"),
        ("description", "一句话描述"),
        ("keywords", "rust"),
        ("siteUrl", "http://test.local"),
        ("pageSize", "1"),
        ("timezone", "0"),
        ("frontPage", "recent"),
        ("frontArchive", "0"),
        ("postDateFormat", "Y-m-d"),
        ("feedFullText", "1"),
        ("commentsRequireMail", "1"),
        ("commentsRequireURL", "0"),
        ("commentsRequireModeration", "0"),
        ("commentsWhitelist", "0"),
        ("commentsCheckReferer", "1"),
        ("commentsPostIntervalEnable", "0"),
        ("commentsPostInterval", "60"),
        ("commentsMarkdown", "0"),
        ("commentsThreaded", "1"),
        ("theme:default", theme),
    ];
    for (name, value) in options {
        sqlx::query(r#"INSERT INTO "typecho_options" ("name","user","value") VALUES (?, 0, ?)"#)
            .bind(name)
            .bind(value)
            .execute(&db.pool)
            .await
            .unwrap();
    }
}

async fn call(
    app: &axum::Router,
    req: Request<Body>,
) -> (StatusCode, String, axum::http::HeaderMap) {
    let res = app.clone().oneshot(req).await.expect("oneshot");
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.expect("body").to_bytes();
    (
        status,
        String::from_utf8_lossy(&bytes).into_owned(),
        headers,
    )
}

async fn get(app: &axum::Router, path: &str) -> (StatusCode, String, axum::http::HeaderMap) {
    call(
        app,
        Request::builder().uri(path).body(Body::empty()).unwrap(),
    )
    .await
}

async fn post(
    app: &axum::Router,
    path: &str,
    form: &str,
    referer: Option<&str>,
) -> (StatusCode, String, axum::http::HeaderMap) {
    let mut b = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(referer) = referer {
        b = b.header("referer", referer);
    }
    call(app, b.body(Body::from(form.to_string())).unwrap()).await
}

fn assert_ok(label: &str, status: StatusCode, body: &str) {
    assert_eq!(status, StatusCode::OK, "{label}\n{body}");
}

#[tokio::test]
async fn public_pages_follow_typecho_12() {
    let (app, db, _uploads) = boot().await;

    let (status, body, _) = get(&app, "/").await;
    assert_ok("home", status, &body);
    assert!(body.contains("第二篇"), "{body}");
    assert!(body.contains("自定义摘要"), "{body}");
    assert!(body.contains("id=\"nav-menu\""), "{body}");
    assert!(body.contains("id=\"secondary\""), "{body}");
    assert!(body.contains("最新文章"), "{body}");
    assert!(body.contains("/page/2"), "{body}");
    assert!(body.contains("post-title"), "{body}");
    assert!(body.contains("/usr/themes/default/assets/css/style.css"), "{body}");
    assert!(!body.contains("隐藏文"), "{body}");
    assert!(!body.contains("未来文"), "{body}");

    let (status, body, _) = get(&app, "/page/2").await;
    assert_ok("page2", status, &body);
    assert!(body.contains("你好世界"), "{body}");
    assert!(body.contains("第一段"), "{body}");
    assert!(!body.contains("秘密"), "{body}");

    let (status, body, _) = get(&app, "/page/3").await;
    assert_ok("page3", status, &body);
    assert!(body.contains("此内容被密码保护"), "{body}");

    let (status, body, _) = get(&app, "/page/4").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, body, _) = get(&app, "/archives/1").await;
    assert_ok("post cid", status, &body);
    assert!(body.contains("秘密段落") || body.contains("秘密"), "{body}");
    assert!(body.contains("早已在此"), "{body}");
    assert!(body.contains("上一篇"), "{body}");
    assert!(body.contains("第二篇"), "{body}");

    let (status, body, _) = get(&app, "/archives/hello-world").await;
    assert_ok("slug", status, &body);
    assert!(body.contains("你好世界"), "{body}");
    let (status, body, _) = get(&app, "/archives/hello-world.html").await;
    assert_ok("slug html", status, &body);
    assert!(body.contains("你好世界"), "{body}");
    let (status, body, _) = get(&app, "/index.php/archives/1").await;
    assert_ok("index.php", status, &body);
    assert!(body.contains("早已在此"), "{body}");

    let (status, body, _) = get(&app, "/archives/2").await;
    assert_ok("newer", status, &body);
    assert!(body.contains("下一篇"), "{body}");
    assert!(body.contains("你好世界"), "{body}");

    let (status, body, _) = get(&app, "/category/notes").await;
    assert_ok("category", status, &body);
    assert!(body.contains("第二篇"), "{body}");
    assert!(body.contains("随笔"), "{body}");
    let (status, body, _) = get(&app, "/tag/rust").await;
    assert_ok("tag", status, &body);
    assert!(body.contains("你好世界"), "{body}");

    let (status, body, _) = get(&app, "/search/%E7%A7%98%E5%AF%86").await;
    assert_ok("search", status, &body);
    assert!(body.contains("你好世界"), "{body}");
    assert!(!body.contains("密码正文"), "{body}");

    let (status, _, headers) = get(&app, "/search/?s=%E7%A7%98%E5%AF%86").await;
    assert_eq!(status, StatusCode::FOUND);
    let loc = headers.get("location").unwrap().to_str().unwrap();
    assert!(loc.contains("/search/"), "{loc}");

    let (status, body, headers) = get(&app, "/feed").await;
    assert_ok("feed", status, &body);
    assert!(headers
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .contains("rss"));
    assert!(
        body.contains("你好世界") && body.contains("第二篇"),
        "{body}"
    );
    assert!(
        !body.contains("密码文") && !body.contains("隐藏文") && !body.contains("未来文"),
        "{body}"
    );

    let (status, body, headers) = get(&app, "/feed/atom").await;
    assert_ok("atom", status, &body);
    assert!(headers
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .contains("atom"));
    assert!(body.contains("<feed"), "{body}");

    let (status, body, _) = get(&app, "/feed/archives/1").await;
    assert_ok("post feed", status, &body);
    assert!(body.contains("早已在此"), "{body}");

    let (status, body, _) = get(&app, "/about.html").await;
    assert_ok("page", status, &body);
    assert!(body.contains("关于本站"), "{body}");

    let (status, body, _) = get(&app, "/author/1").await;
    assert_ok("author", status, &body);
    assert!(body.contains("站长"), "{body}");
    assert!(body.contains("第二篇"), "{body}");

    let (status, body, _) = get(&app, "/2023/11/14/").await;
    assert_ok("day", status, &body);
    assert!(body.contains("第二篇"), "{body}");

    let (status, body, _) = get(&app, "/archives/4").await;
    assert_ok("hidden", status, &body);
    assert!(body.contains("隐藏正文"), "{body}");

    let (status, body, _) = get(&app, "/archives/5").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, body, _) = get(&app, "/no-such").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(body.contains("404"), "{body}");

    let (status, body, _) = get(&app, "/blog/").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "blog archive {body}");

    let (status, body, _) = get(&app, "/archives/3").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("请输入密码访问"), "{body}");
    assert!(!body.contains("密码正文"), "{body}");

    let (status, body, _) = post(
        &app,
        "/archives/3",
        "protectPassword=nope&protectCID=3",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("密码错误"), "{body}");

    let (status, body, headers) = post(
        &app,
        "/archives/3",
        "protectPassword=s3cret&protectCID=3",
        None,
    )
    .await;
    assert_ok("unlock", status, &body);
    assert!(body.contains("密码正文"), "{body}");
    assert!(headers.get("set-cookie").is_some());
    let cookie = headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect::<Vec<_>>()
        .join(";");
    let prefix = tiebidli::auth::cookie_prefix("http://test.local");
    assert!(
        cookie.contains(&format!("{prefix}protectPassword_3")),
        "{cookie}"
    );

    let (status, body, _) = post(
        &app,
        "/archives/1/comment",
        "author=&mail=a@b.co&text=你好评论&parent=0",
        Some("http://test.local/archives/1/"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("必须填写用户名"), "{body}");

    let (status, body, _) = post(
        &app,
        "/archives/1/comment",
        "author=站长&mail=a@b.co&text=你好评论&parent=0",
        Some("http://test.local/archives/1/"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("已经被注册"), "{body}");

    let (status, _, headers) = post(
        &app,
        "/archives/1/comment",
        "author=路人&mail=a@b.co&text=你好评论&parent=0",
        Some("http://test.local/archives/1/"),
    )
    .await;
    assert_eq!(status, StatusCode::FOUND, "comment redirect");
    let loc = headers.get("location").unwrap().to_str().unwrap();
    assert!(loc.contains("#comment-"), "{loc}");
    let (status, body, _) = get(&app, "/archives/1").await;
    assert_ok("comment visible", status, &body);
    assert!(body.contains("你好评论"), "{body}");

    sqlx::query(
        r#"UPDATE "typecho_options" SET value = ? WHERE name = 'commentsPostIntervalEnable'"#,
    )
    .bind("1")
    .execute(&db.pool)
    .await
    .unwrap();
    let ten_ago = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        - 10;
    sqlx::query(r#"UPDATE "typecho_comments" SET created = ? WHERE cid = 1"#)
        .bind(ten_ago)
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body, _) = post(
        &app,
        "/archives/1/comment",
        "author=路人乙&mail=b@b.co&text=太快了&parent=0",
        Some("http://test.local/archives/1/"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(body.contains("过于频繁"), "{body}");

    let route = r#"a:1:{s:4:"post";a:1:{s:3:"url";s:21:"/archives/[slug].html";}}"#;
    sqlx::query(
        r#"INSERT INTO "typecho_options" ("name","user","value") VALUES ('routingTable', 0, ?)"#,
    )
    .bind(route)
    .execute(&db.pool)
    .await
    .unwrap();
    let (status, body, _) = get(&app, "/archives/hello-world.html").await;
    assert_ok("custom permalink", status, &body);
    assert!(
        body.contains("hello-world.html"),
        "custom permalink missing from rendered links"
    );
    let (status, body, _) = get(&app, "/archives/1").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "cid permalink retired\n{body}"
    );

    sqlx::query(r#"UPDATE "typecho_options" SET value = 'page:6' WHERE name = 'frontPage'"#)
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body, _) = get(&app, "/").await;
    assert_ok("static front", status, &body);
    assert!(body.contains("关于本站"), "{body}");
    let (status, body, _) = get(&app, "/page/2").await;
    assert_ok("static front page 2", status, &body);
    assert!(body.contains("关于本站"), "{body}");
    let (status, _, _) = get(&app, "/blog/").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    sqlx::query(r#"UPDATE "typecho_options" SET value = '1' WHERE name = 'frontArchive'"#)
        .execute(&db.pool)
        .await
        .unwrap();
    let (status, body, _) = get(&app, "/blog/").await;
    assert_ok("front archive", status, &body);
    assert!(body.contains("第二篇"), "{body}");
}

#[tokio::test]
async fn feeds_comments_admin_and_xmlrpc() {
    let (app, db, uploads) = boot().await;
    let prefix = tiebidli::auth::cookie_prefix("http://test.local");

    let (status, body, headers) = get(&app, "/feed/rss").await;
    assert_ok("rss1", status, &body);
    assert!(
        headers
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("rdf"),
        "{headers:?}"
    );
    assert!(body.contains("rdf:RDF"), "{body}");
    assert!(body.contains("你好世界"), "{body}");

    let (status, body, _) = get(&app, "/feed/atom").await;
    assert_ok("atom still", status, &body);
    assert!(body.contains("<feed"), "{body}");

    sqlx::query(
        r#"INSERT INTO "typecho_options" ("name","user","value") VALUES
           ('commentsPageBreak', 0, '1'),
           ('commentsPageSize', 0, '1')"#,
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        r#"INSERT INTO "typecho_comments"
        ("cid","created","author","authorId","ownerId","mail","url","ip","agent","text","type","status","parent")
        VALUES (1, 1700000200, '后来的人', 0, 1, 'new@example.com', NULL, '127.0.0.1', 'test', '第二页评论', 'comment', 'approved', 0)"#,
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let (status, body, _) = get(&app, "/archives/1/comment-page-2").await;
    assert_ok("comment page", status, &body);
    assert!(body.contains("第二页评论"), "{body}");
    assert!(body.contains("comment-page-nav"), "{body}");
    assert!(body.contains("class=\"current\""), "{body}");

    for (cid, title, status_name) in [(21_i64, "私人文章", "private"), (22, "草稿文章", "draft")] {
        sqlx::query(
            r#"INSERT INTO "typecho_contents"
            ("cid","title","slug","created","modified","text","order","authorId","template","type","status","password","commentsNum","allowComment","allowPing","allowFeed","parent")
            VALUES (?, ?, ?, 1700000000, 1700000000, '不可见', 0, 1, NULL, 'post', ?, NULL, 0, '1', '1', '1', 0)"#,
        )
        .bind(cid)
        .bind(title)
        .bind(format!("hidden-{cid}"))
        .bind(status_name)
        .execute(&db.pool)
        .await
        .unwrap();
        let (status, body, _) = get(&app, &format!("/archives/{cid}")).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{title} {body}");
    }
    let (status, body, _) = get(&app, "/").await;
    assert_ok("home hides private", status, &body);
    assert!(!body.contains("私人文章") && !body.contains("草稿文章"), "{body}");

    std::fs::write(uploads.join("note.txt"), b"file-body").unwrap();
    let attach = tiebidli::write::attachment_text(
        "note.txt",
        "usr/uploads/note.txt",
        9,
        "txt",
        "text/plain",
    );
    sqlx::query(
        r#"INSERT INTO "typecho_contents"
        ("cid","title","slug","created","modified","text","order","authorId","template","type","status","password","commentsNum","allowComment","allowPing","allowFeed","parent")
        VALUES (30, '附件说明', 'attach-note', 1700000000, 1700000000, ?, 0, 1, NULL, 'attachment', 'publish', NULL, 0, '0', '0', '0', 0)"#,
    )
    .bind(attach)
    .execute(&db.pool)
    .await
    .unwrap();
    let (status, body, _) = get(&app, "/attachment/30").await;
    assert_ok("attachment", status, &body);
    assert!(body.contains("note.txt"), "{body}");
    let (status, body, headers) = get(&app, "/attachment/30?download=1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("file-body"), "{body}");
    assert!(
        headers
            .get("content-disposition")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .contains("note.txt"),
        "{headers:?}"
    );

    let (status, body, _) = post(
        &app,
        "/archives/1/trackback",
        "blog_name=Example&url=http://example.com/post&excerpt=引用摘要",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("<success>0</success>"), "{body}");
    assert!(body.contains("Trackback has registered."), "{body}");
    let (status, body, _) = get(&app, "/archives/1").await;
    assert_ok("ping list", status, &body);
    assert!(body.contains("引用摘要"), "{body}");
    assert!(body.contains("ping-list"), "{body}");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let Ok((mut sock, _)) = listener.accept().await else {
            return;
        };
        let mut buf = [0u8; 2048];
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let _ = sock.read(&mut buf).await;
        let page = "<html><title>来源页</title><a href=\"http://test.local/archives/1/\">链接</a></html>";
        let resp = format!(
            "HTTP/1.0 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
            page.len()
        );
        let _ = sock.write_all(resp.as_bytes()).await;
    });
    let source = format!("http://127.0.0.1:{port}/source");
    let ping_xml = format!(
        r#"<?xml version="1.0"?><methodCall><methodName>pingback.ping</methodName><params><param><value><string>{source}</string></value></param><param><value><string>http://test.local/archives/1/</string></value></param></params></methodCall>"#
    );
    let (status, body, _) = call(
        &app,
        Request::builder()
            .method("POST")
            .uri("/action/xmlrpc")
            .header("content-type", "text/xml")
            .body(Body::from(ping_xml))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!body.contains("fault"), "{body}");

    let stored = tiebidli::auth::md5_hex(b"adminpass");
    sqlx::query(r#"UPDATE "typecho_users" SET password = ? WHERE name = 'admin'"#)
        .bind(stored)
        .execute(&db.pool)
        .await
        .unwrap();
    let xml = r#"<?xml version="1.0"?><methodCall><methodName>metaWeblog.newPost</methodName><params><param><value><string>1</string></value></param><param><value><string>admin</string></value></param><param><value><string>adminpass</string></value></param><param><value><struct><member><name>title</name><value><string>接口文章</string></value></member><member><name>description</name><value><string>接口正文</string></value></member><member><name>dateCreated</name><value><dateTime.iso8601>20200101T00:00:00</dateTime.iso8601></value></member></struct></value></param><param><value><boolean>1</boolean></value></param></params></methodCall>"#;
    let (status, body, _) = call(
        &app,
        Request::builder()
            .method("POST")
            .uri("/action/xmlrpc")
            .header("content-type", "text/xml")
            .body(Body::from(xml))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("<int>"), "{body}");
    let cid = body
        .split("<int>")
        .nth(1)
        .and_then(|s| s.split("</int>").next())
        .unwrap_or("0");
    let (status, body, _) = get(&app, &format!("/archives/{cid}")).await;
    assert_ok("xmlrpc post", status, &body);
    assert!(body.contains("接口正文"), "{body}");

    let (status, _, headers) = post(
        &app,
        "/admin/login",
        "name=admin&password=adminpass",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FOUND, "login");
    let cookie = headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or(v).to_string())
        .collect::<Vec<_>>()
        .join("; ");
    assert!(cookie.contains(&format!("{prefix}__typecho_uid")), "{cookie}");
    let (status, body, _) = call(
        &app,
        Request::builder()
            .method("POST")
            .uri("/admin/posts/2")
            .header("content-type", "application/x-www-form-urlencoded")
            .header("cookie", cookie)
            .body(Body::from(
                "title=改过的标题&slug=second-post&text=修改后的正文&status=publish&created=1700001000&allowComment=on&allowPing=on&allowFeed=on",
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::FOUND, "{body}");
    let (status, body, _) = get(&app, "/archives/2").await;
    assert_ok("edited post", status, &body);
    assert!(body.contains("改过的标题"), "{body}");
    assert!(body.contains("修改后的正文"), "{body}");
}

#[tokio::test]
async fn admin_profile_comments_preview_and_fields() {
    let (app, db, _uploads) = boot().await;
    let stored = tiebidli::auth::md5_hex(b"adminpass");
    sqlx::query(r#"UPDATE "typecho_users" SET password = ? WHERE name = 'admin'"#)
        .bind(stored)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        r#"INSERT INTO "typecho_users"
        ("uid","name","password","mail","url","screenName","created","activated","logged","group","authCode")
        VALUES (2, 'other', 'x', 'other@example.com', '', '另一人', 1600000000, 1, 0, 'subscriber', NULL)"#,
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let (status, _, headers) = post(&app, "/admin/login", "name=admin&password=adminpass", None).await;
    assert_eq!(status, StatusCode::FOUND, "login");
    let cookie = headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or(v).to_string())
        .collect::<Vec<_>>()
        .join("; ");

    let (status, body, _) = authed(&app, "GET", "/admin", &cookie, "").await;
    assert_ok("dashboard author link", status, &body);
    assert!(body.contains("href=\"/admin/profile\""), "{body}");
    assert!(body.contains("个人设置"), "{body}");

    let (status, body, _) = authed(&app, "GET", "/admin/profile", &cookie, "").await;
    assert_ok("profile", status, &body);
    assert!(body.contains("个人资料"), "{body}");
    assert!(body.contains("撰写设置"), "{body}");
    assert!(body.contains("密码修改"), "{body}");
    assert!(body.contains("插件个人设置"), "{body}");
    assert!(body.contains("更新我的档案"), "{body}");
    assert!(body.contains("使用 Markdown 语法编辑和解析内容"), "{body}");
    assert!(body.contains("可以被评论"), "{body}");

    let (status, _, headers) = authed(
        &app,
        "POST",
        "/admin/profile",
        &cookie,
        "screenName=站长&url=https://example.com&mail=&do=profile",
    )
    .await;
    assert_eq!(status, StatusCode::FOUND);
    let (status, body, _) = authed(&app, "GET", &location_of(&headers), &cookie, "").await;
    assert_ok("empty mail", status, &body);
    assert!(body.contains("必须填写电子邮箱"), "{body}");

    let (status, _, headers) = authed(
        &app,
        "POST",
        "/admin/profile",
        &cookie,
        "screenName=站长&url=https://example.com&mail=not-an-email&do=profile",
    )
    .await;
    assert_eq!(status, StatusCode::FOUND);
    let (status, body, _) = authed(&app, "GET", &location_of(&headers), &cookie, "").await;
    assert_ok("bad mail", status, &body);
    assert!(body.contains("电子邮箱格式错误"), "{body}");

    let (status, _, headers) = authed(
        &app,
        "POST",
        "/admin/profile",
        &cookie,
        "screenName=%3Cb%3Ex%3C%2Fb%3E&url=https://example.com&mail=admin@example.com&do=profile",
    )
    .await;
    assert_eq!(status, StatusCode::FOUND);
    let (status, body, _) = authed(&app, "GET", &location_of(&headers), &cookie, "").await;
    assert_ok("xss screen", status, &body);
    assert!(body.contains("请不要在昵称中使用特殊字符"), "{body}");

    let (status, _, headers) = authed(
        &app,
        "POST",
        "/admin/profile",
        &cookie,
        "screenName=%E7%AB%99%E9%95%BF&url=javascript:alert(1)&mail=admin@example.com&do=profile",
    )
    .await;
    assert_eq!(status, StatusCode::FOUND);
    let (status, body, _) = authed(&app, "GET", &location_of(&headers), &cookie, "").await;
    assert_ok("bad url", status, &body);
    assert!(body.contains("个人主页地址格式错误"), "{body}");

    let (status, _, headers) = authed(
        &app,
        "POST",
        "/admin/profile",
        &cookie,
        "screenName=%E5%8F%A6%E4%B8%80%E4%BA%BA&url=https://example.com&mail=admin@example.com&do=profile",
    )
    .await;
    assert_eq!(status, StatusCode::FOUND);
    let (status, body, _) = authed(&app, "GET", &location_of(&headers), &cookie, "").await;
    assert_ok("dup screen", status, &body);
    assert!(body.contains("昵称已经存在"), "{body}");

    let (status, _, headers) = authed(
        &app,
        "POST",
        "/admin/profile",
        &cookie,
        "screenName=%E6%96%B0%E7%AB%99%E9%95%BF&url=https://example.com/me&mail=admin@example.com&do=profile",
    )
    .await;
    assert_eq!(status, StatusCode::FOUND);
    let (status, body, _) = authed(&app, "GET", &location_of(&headers), &cookie, "").await;
    assert_ok("profile saved", status, &body);
    assert!(body.contains("您的档案已经更新"), "{body}");
    assert!(body.contains("value=\"新站长\""), "{body}");

    let (status, _, headers) = authed(
        &app,
        "POST",
        "/admin/profile",
        &cookie,
        "markdown=1&xmlrpcMarkdown=0&autoSave=1&defaultAllow%5B%5D=comment&do=options",
    )
    .await;
    assert_eq!(status, StatusCode::FOUND);
    let (status, body, _) = authed(&app, "GET", &location_of(&headers), &cookie, "").await;
    assert_ok("options saved", status, &body);
    assert!(body.contains("设置已经保存"), "{body}");
    let markdown: String = sqlx::query_scalar(
        r#"SELECT value FROM "typecho_options" WHERE name = 'markdown' AND user = 1"#,
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(markdown, "1");
    let ping: String = sqlx::query_scalar(
        r#"SELECT value FROM "typecho_options" WHERE name = 'defaultAllowPing' AND user = 1"#,
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(ping, "0");

    let (status, _, headers) = authed(
        &app,
        "POST",
        "/admin/profile",
        &cookie,
        "password=short&confirm=short&do=password",
    )
    .await;
    assert_eq!(status, StatusCode::FOUND);
    let (status, body, _) = authed(&app, "GET", &location_of(&headers), &cookie, "").await;
    assert_ok("short password", status, &body);
    assert!(body.contains("为了保证账户安全, 请输入至少六位的密码"), "{body}");

    let (status, _, headers) = authed(
        &app,
        "POST",
        "/admin/profile",
        &cookie,
        "password=longpass&confirm=otherpass&do=password",
    )
    .await;
    assert_eq!(status, StatusCode::FOUND);
    let (status, body, _) = authed(&app, "GET", &location_of(&headers), &cookie, "").await;
    assert_ok("password mismatch", status, &body);
    assert!(body.contains("两次输入的密码不一致"), "{body}");

    let (status, _, headers) = authed(
        &app,
        "POST",
        "/admin/profile",
        &cookie,
        "password=longpass&confirm=longpass&do=password",
    )
    .await;
    assert_eq!(status, StatusCode::FOUND);
    let (status, body, _) = authed(&app, "GET", &location_of(&headers), &cookie, "").await;
    assert_ok("password saved", status, &body);
    assert!(body.contains("密码已经成功修改"), "{body}");
    let (status, _, headers) = post(&app, "/admin/login", "name=admin&password=longpass", None).await;
    assert_eq!(status, StatusCode::FOUND, "new password");
    let cookie = headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or(v).to_string())
        .collect::<Vec<_>>()
        .join("; ");

    let (status, body, _) = authed(&app, "GET", "/admin/comments", &cookie, "").await;
    assert_ok("comments", status, &body);
    assert!(body.contains("class=\"operate-edit\""), "{body}");
    assert!(body.contains("class=\"operate-reply\""), "{body}");
    assert!(body.contains("data-comment="), "{body}");
    let (status, body, _) = authed(
        &app,
        "POST",
        "/admin/comments/edit?coid=1",
        &cookie,
        "author=%E6%94%B9%E4%BD%9C%E8%80%85&mail=new%40example.com&url=https%3A%2F%2Fex.com&text=%E6%94%B9%E8%BF%87%E7%9A%84%E8%AF%84%E8%AE%BA",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"success\":1"), "{body}");
    assert!(body.contains("改过的评论"), "{body}");
    let text: String = sqlx::query_scalar(r#"SELECT text FROM "typecho_comments" WHERE coid = 1"#)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(text, "改过的评论");

    let (status, body, _) = authed(
        &app,
        "POST",
        "/admin/comments/reply?coid=1",
        &cookie,
        "text=%E7%AE%A1%E7%90%86%E5%91%98%E5%9B%9E%E5%A4%8D",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"success\":1"), "{body}");
    let (parent, author): (i64, String) = sqlx::query_as(
        r#"SELECT parent, author FROM "typecho_comments" WHERE text = '管理员回复'"#,
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(parent, 1);
    assert_eq!(author, "新站长");

    let before: i64 = sqlx::query_scalar(r#"SELECT COUNT(*) FROM "typecho_contents""#)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let (status, body, _) = authed(
        &app,
        "POST",
        "/admin/preview",
        &cookie,
        "title=%E9%A2%84%E8%A7%88%E6%9C%AA%E4%BF%9D%E5%AD%98%E6%A0%87%E9%A2%98&text=%E9%A2%84%E8%A7%88%E6%9C%AA%E4%BF%9D%E5%AD%98%E6%AD%A3%E6%96%87XYZ&slug=not-saved&type=post",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("预览未保存标题"), "{body}");
    assert!(body.contains("预览未保存正文XYZ"), "{body}");
    assert!(body.contains("id=\"nav-menu\""), "{body}");
    let after: i64 = sqlx::query_scalar(r#"SELECT COUNT(*) FROM "typecho_contents""#)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    let (status, home, _) = get(&app, "/").await;
    assert_ok("home unchanged", status, &home);
    assert!(!home.contains("预览未保存标题"), "{home}");

    let (status, body, _) = authed(&app, "GET", "/admin/posts/new", &cookie, "").await;
    assert_ok("write form", status, &body);
    assert!(body.contains("预览文章"), "{body}");
    assert!(body.contains("自定义字段"), "{body}");
    assert!(body.contains("引用通告"), "{body}");
    assert!(body.contains("name=\"trackback\""), "{body}");

    let (status, _, _) = authed(
        &app,
        "POST",
        "/admin/posts/2",
        &cookie,
        "title=%E7%AC%AC%E4%BA%8C%E7%AF%87&slug=second-post&text=%E6%AD%A3%E6%96%87&do=publish&visibility=publish&created=1700001000&allowComment=1&allowPing=1&allowFeed=1&fieldNames%5B%5D=customSummary&fieldTypes%5B%5D=str&fieldValues%5B%5D=%E6%91%98%E8%A6%81%E4%B9%99&fieldNames%5B%5D=score&fieldTypes%5B%5D=int&fieldValues%5B%5D=7",
    )
    .await;
    assert_eq!(status, StatusCode::FOUND);
    let summary: String = sqlx::query_scalar(
        r#"SELECT str_value FROM "typecho_fields" WHERE cid = 2 AND name = 'customSummary'"#,
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(summary, "摘要乙");
    let score: i64 = sqlx::query_scalar(
        r#"SELECT int_value FROM "typecho_fields" WHERE cid = 2 AND name = 'score'"#,
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(score, 7);
}

fn location_of(headers: &axum::http::HeaderMap) -> String {
    headers
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("/admin/profile")
        .to_string()
}

async fn authed(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: &str,
    form: &str,
) -> (StatusCode, String, axum::http::HeaderMap) {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("cookie", cookie);
    if method == "POST" {
        b = b.header("content-type", "application/x-www-form-urlencoded");
    }
    let body = if method == "POST" {
        Body::from(form.to_string())
    } else {
        Body::empty()
    };
    call(app, b.body(body).unwrap()).await
}
