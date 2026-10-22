use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tiebidli::db::Db;
use tiebidli::{build_router, AppState, Config};
use tower::ServiceExt;

static N: AtomicU64 = AtomicU64::new(0);

async fn boot() -> (axum::Router, Db) {
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
    let cfg = Config::for_test(&url);
    let marker = cfg.theme_dir.join("assets/css/handsome.min.css");
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
    (app, db)
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
        ("theme:handsome", theme),
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
    let (app, db) = boot().await;

    let (status, body, _) = get(&app, "/").await;
    assert_ok("home", status, &body);
    assert!(body.contains("第二篇"), "{body}");
    assert!(body.contains("自定义摘要"), "{body}");
    assert!(body.contains("写点东西"), "{body}");
    assert!(body.contains("/page/2"), "{body}");
    assert!(body.contains("index-post-title"), "{body}");
    assert!(
        body.contains("THEME_POST_CONTENT: \"2\""),
        "handsome script expects this constant\n{body}"
    );
    assert!(body.contains("SHOW_FOOTER: \"1\""), "{body}");
    assert!(body.contains("PLUGIN_READY: \"1\""), "{body}");
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
