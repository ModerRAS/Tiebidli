//! 后台和 XML-RPC 共用的写入。列名对齐 Typecho 1.2，不改表结构。

use crate::db::{col_i64, col_str, is_yes, Db};
use crate::model::{Comment, Meta, Post};
use crate::phpval::PhpValue;
use crate::repo::NewComment;
use std::collections::HashMap;

pub struct ContentInput {
    pub cid: Option<i64>,
    pub title: String,
    pub slug: String,
    pub text: String,
    pub created: i64,
    pub kind: String,
    pub status: String,
    pub password: String,
    pub allow_comment: bool,
    pub allow_ping: bool,
    pub allow_feed: bool,
    pub author_id: i64,
    pub parent: i64,
    pub sort_order: i64,
    pub categories: Vec<i64>,
    pub tags: Vec<String>,
}

pub struct MetaInput {
    pub mid: Option<i64>,
    pub name: String,
    pub slug: String,
    pub kind: String,
    pub description: String,
    pub parent: i64,
}

pub struct FieldInput {
    pub name: String,
    pub kind: String,
    pub value: String,
}

pub struct FieldRow {
    pub name: String,
    pub kind: String,
    pub str_value: String,
    pub int_value: i64,
    pub float_text: String,
}

pub struct UserInput {
    pub uid: Option<i64>,
    pub name: String,
    pub password: String,
    pub mail: String,
    pub url: String,
    pub screen: String,
    pub group: String,
    pub created: i64,
}

impl Db {
    pub async fn save_content(&self, input: &ContentInput) -> Result<i64, sqlx::Error> {
        let now = chrono::Utc::now().timestamp();
        let title = trunc(&input.title, 150);
        let password = trunc(&input.password, 32);
        let kind = if input.kind == "page" || input.kind == "attachment" {
            input.kind.as_str()
        } else {
            "post"
        };
        let status = normalize_status(&input.status);
        let yes = |b: bool| if b { "1" } else { "0" };
        let order = self.order_col();
        let cid = if let Some(cid) = input.cid {
            let sql = format!(
                "UPDATE {} SET title = ?, slug = ?, created = ?, modified = ?, text = ?, {order} = ?, \
                 authorId = ?, type = ?, status = ?, password = ?, allowComment = ?, allowPing = ?, \
                 allowFeed = ?, parent = ? WHERE cid = ?",
                self.table("contents")
            );
            sqlx::query(&sql)
                .bind(&title)
                .bind(trunc(&input.slug, 150))
                .bind(input.created)
                .bind(now)
                .bind(&input.text)
                .bind(input.sort_order)
                .bind(input.author_id)
                .bind(kind)
                .bind(status)
                .bind(empty_null(&password))
                .bind(yes(input.allow_comment))
                .bind(yes(input.allow_ping))
                .bind(yes(input.allow_feed))
                .bind(input.parent)
                .bind(cid)
                .execute(&self.pool)
                .await?;
            cid
        } else {
            let sql = format!(
                "INSERT INTO {} (title, slug, created, modified, text, {order}, authorId, template, type, status, password, commentsNum, allowComment, allowPing, allowFeed, parent) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?, 0, ?, ?, ?, ?)",
                self.table("contents")
            );
            let slug = if input.slug.trim().is_empty() {
                format!("tmp-{now}")
            } else {
                trunc(&input.slug, 150)
            };
            let res = sqlx::query(&sql)
                .bind(&title)
                .bind(&slug)
                .bind(input.created)
                .bind(now)
                .bind(&input.text)
                .bind(input.sort_order)
                .bind(input.author_id)
                .bind(kind)
                .bind(status)
                .bind(empty_null(&password))
                .bind(yes(input.allow_comment))
                .bind(yes(input.allow_ping))
                .bind(yes(input.allow_feed))
                .bind(input.parent)
                .execute(&self.pool)
                .await?;
            last_id(self, res.last_insert_id().unwrap_or(0), "contents", "cid").await?
        };
        let slug = unique_slug(self, cid, &input.slug, &title).await?;
        let sql = format!(
            "UPDATE {} SET slug = ? WHERE cid = ?",
            self.table("contents")
        );
        sqlx::query(&sql)
            .bind(&slug)
            .bind(cid)
            .execute(&self.pool)
            .await?;
        if kind != "attachment" {
            replace_terms(self, cid, &input.categories, &input.tags).await?;
        }
        Ok(cid)
    }

    pub async fn set_content_status(&self, cid: i64, status: &str) -> Result<(), sqlx::Error> {
        let status = match status {
            "publish" | "hidden" | "private" | "waiting" | "draft" => status,
            _ => return Ok(()),
        };
        let sql = format!("UPDATE {} SET status = ? WHERE cid = ?", self.table("contents"));
        sqlx::query(&sql)
            .bind(status)
            .bind(cid)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_content(&self, cid: i64) -> Result<(), sqlx::Error> {
        let mids = related_mids(self, cid).await?;
        for table in ["relationships", "comments", "fields", "contents"] {
            let col = if table == "contents" { "cid" } else { "cid" };
            let sql = format!("DELETE FROM {} WHERE {col} = ?", self.table(table));
            sqlx::query(&sql).bind(cid).execute(&self.pool).await?;
        }
        for mid in mids {
            recount_meta(self, mid).await?;
        }
        Ok(())
    }

    pub async fn list_contents(&self, kind: &str, limit: i64) -> Result<Vec<Post>, sqlx::Error> {
        let sql = format!(
            "SELECT {cols} FROM {contents} c LEFT JOIN {users} u ON c.authorId = u.uid \
             WHERE c.type = ? ORDER BY c.created DESC, c.cid DESC LIMIT ?",
            cols = content_cols(self),
            contents = self.table("contents"),
            users = self.table("users"),
        );
        let rows = sqlx::query(&sql)
            .bind(kind)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        let mut posts = Vec::new();
        for row in &rows {
            posts.push(map_post_row(row)?);
        }
        attach_terms(self, &mut posts).await?;
        Ok(posts)
    }

    pub async fn save_meta(&self, input: &MetaInput) -> Result<i64, sqlx::Error> {
        let name = trunc(&input.name, 150);
        let slug = {
            let raw = if input.slug.trim().is_empty() {
                slugify(&name)
            } else {
                slugify(&input.slug)
            };
            if raw.is_empty() {
                format!("m-{}", chrono::Utc::now().timestamp())
            } else {
                trunc(&raw, 150)
            }
        };
        let order = self.order_col();
        if let Some(mid) = input.mid {
            let sql = format!(
                "UPDATE {} SET name = ?, slug = ?, type = ?, description = ?, parent = ? WHERE mid = ?",
                self.table("metas")
            );
            sqlx::query(&sql)
                .bind(&name)
                .bind(&slug)
                .bind(&input.kind)
                .bind(trunc(&input.description, 150))
                .bind(input.parent)
                .bind(mid)
                .execute(&self.pool)
                .await?;
            let _ = order;
            Ok(mid)
        } else {
            let sql = format!(
                "INSERT INTO {} (name, slug, type, description, count, {order}, parent) VALUES (?, ?, ?, ?, 0, 0, ?)",
                self.table("metas")
            );
            let res = sqlx::query(&sql)
                .bind(&name)
                .bind(&slug)
                .bind(&input.kind)
                .bind(trunc(&input.description, 150))
                .bind(input.parent)
                .execute(&self.pool)
                .await?;
            last_id(self, res.last_insert_id().unwrap_or(0), "metas", "mid").await
        }
    }

    pub async fn delete_meta(&self, mid: i64) -> Result<(), sqlx::Error> {
        let sql = format!("DELETE FROM {} WHERE mid = ?", self.table("relationships"));
        sqlx::query(&sql).bind(mid).execute(&self.pool).await?;
        let sql = format!("DELETE FROM {} WHERE mid = ?", self.table("metas"));
        sqlx::query(&sql).bind(mid).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn list_metas(&self, kind: &str) -> Result<Vec<Meta>, sqlx::Error> {
        let order = self.order_col();
        let sql = format!(
            "SELECT mid, name, slug, type, description, count, {order} AS sort_order, parent \
             FROM {} WHERE type = ? ORDER BY {order} ASC, mid ASC",
            self.table("metas")
        );
        let rows = sqlx::query(&sql).bind(kind).fetch_all(&self.pool).await?;
        rows.iter().map(map_meta_row).collect()
    }

    pub async fn set_comment_status(&self, coid: i64, status: &str) -> Result<(), sqlx::Error> {
        let status = match status {
            "approved" | "waiting" | "spam" => status,
            _ => "waiting",
        };
        let sql = format!(
            "UPDATE {} SET status = ? WHERE coid = ?",
            self.table("comments")
        );
        sqlx::query(&sql)
            .bind(status)
            .bind(coid)
            .execute(&self.pool)
            .await?;
        if let Some(cid) = comment_cid(self, coid).await? {
            refresh_comment_count(self, cid).await?;
        }
        Ok(())
    }

    pub async fn delete_comment(&self, coid: i64) -> Result<(), sqlx::Error> {
        let cid = comment_cid(self, coid).await?;
        let sql = format!("DELETE FROM {} WHERE coid = ?", self.table("comments"));
        sqlx::query(&sql).bind(coid).execute(&self.pool).await?;
        if let Some(cid) = cid {
            refresh_comment_count(self, cid).await?;
        }
        Ok(())
    }

    pub async fn list_comments(&self, limit: i64) -> Result<Vec<Comment>, sqlx::Error> {
        let sql = format!(
            "SELECT coid, cid, created, author, authorId, ownerId, mail, url, ip, text, type, status, parent \
             FROM {} ORDER BY created DESC, coid DESC LIMIT ?",
            self.table("comments")
        );
        let rows = sqlx::query(&sql).bind(limit).fetch_all(&self.pool).await?;
        Ok(rows.iter().map(map_comment_row).collect())
    }

    pub async fn ping_exists(&self, cid: i64, url: &str) -> Result<bool, sqlx::Error> {
        let n = scalar(
            self,
            &format!(
                "SELECT COUNT(*) AS n FROM {} WHERE cid = ? AND url = ? AND type <> 'comment'",
                self.table("comments")
            ),
            cid,
            Some(url),
        )
        .await?;
        Ok(n > 0)
    }

    pub async fn spam_from_ip(&self, ip: &str) -> Result<bool, sqlx::Error> {
        let sql = format!(
            "SELECT COUNT(*) AS n FROM {} WHERE status = 'spam' AND ip = ?",
            self.table("comments")
        );
        let row = sqlx::query(&sql).bind(ip).fetch_optional(&self.pool).await?;
        Ok(row
            .as_ref()
            .map(|r| col_i64(r, "n").unwrap_or(0))
            .unwrap_or(0)
            > 0)
    }

    pub async fn insert_feedback(&self, c: &NewComment) -> Result<i64, sqlx::Error> {
        self.insert_comment(c).await
    }

    pub async fn user_options(&self, uid: i64) -> Result<HashMap<String, String>, sqlx::Error> {
        let user_col = self.quote("user");
        let sql = format!(
            "SELECT name, value, {user_col} AS owner FROM {} WHERE {user_col} = 0 OR {user_col} = ?",
            self.table("options")
        );
        let rows = sqlx::query(&sql).bind(uid).fetch_all(&self.pool).await?;
        let mut base = HashMap::new();
        let mut own = HashMap::new();
        for row in &rows {
            let slot = if col_i64(row, "owner").unwrap_or(0) == uid {
                &mut own
            } else {
                &mut base
            };
            slot.insert(col_str(row, "name"), col_str(row, "value"));
        }
        base.extend(own);
        Ok(base)
    }

    pub async fn set_user_option(&self, uid: i64, name: &str, value: &str) -> Result<(), sqlx::Error> {
        let name = trunc(name, 32);
        let user_col = self.quote("user");
        let count_sql = format!(
            "SELECT COUNT(*) AS n FROM {} WHERE name = ? AND {user_col} = ?",
            self.table("options")
        );
        let row = sqlx::query(&count_sql)
            .bind(&name)
            .bind(uid)
            .fetch_optional(&self.pool)
            .await?;
        let exists = row
            .as_ref()
            .map(|r| col_i64(r, "n").unwrap_or(0))
            .unwrap_or(0)
            > 0;
        if exists {
            let sql = format!(
                "UPDATE {} SET value = ? WHERE name = ? AND {user_col} = ?",
                self.table("options")
            );
            sqlx::query(&sql)
                .bind(value)
                .bind(&name)
                .bind(uid)
                .execute(&self.pool)
                .await?;
        } else {
            let sql = format!(
                "INSERT INTO {} (name, {user_col}, value) VALUES (?, ?, ?)",
                self.table("options")
            );
            sqlx::query(&sql)
                .bind(&name)
                .bind(uid)
                .bind(value)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    pub async fn screen_taken(&self, uid: i64, screen: &str) -> Result<bool, sqlx::Error> {
        let sql = format!(
            "SELECT COUNT(*) AS n FROM {} WHERE screenName = ? AND uid <> ?",
            self.table("users")
        );
        let row = sqlx::query(&sql)
            .bind(screen)
            .bind(uid)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row
            .as_ref()
            .map(|r| col_i64(r, "n").unwrap_or(0))
            .unwrap_or(0)
            > 0)
    }

    pub async fn mail_taken(&self, uid: i64, mail: &str) -> Result<bool, sqlx::Error> {
        let sql = format!(
            "SELECT COUNT(*) AS n FROM {} WHERE mail = ? AND uid <> ?",
            self.table("users")
        );
        let row = sqlx::query(&sql)
            .bind(mail)
            .bind(uid)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row
            .as_ref()
            .map(|r| col_i64(r, "n").unwrap_or(0))
            .unwrap_or(0)
            > 0)
    }

    pub async fn update_own_profile(
        &self,
        uid: i64,
        screen: &str,
        mail: &str,
        url: &str,
    ) -> Result<(), sqlx::Error> {
        let sql = format!(
            "UPDATE {} SET screenName = ?, mail = ?, url = ? WHERE uid = ?",
            self.table("users")
        );
        sqlx::query(&sql)
            .bind(trunc(screen, 32))
            .bind(trunc(mail, 150))
            .bind(trunc(url, 150))
            .bind(uid)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_password(&self, uid: i64, password: &str) -> Result<(), sqlx::Error> {
        let sql = format!(
            "UPDATE {} SET password = ? WHERE uid = ?",
            self.table("users")
        );
        sqlx::query(&sql)
            .bind(password)
            .bind(uid)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn profile_stats(&self, uid: i64) -> Result<(i64, i64, i64, i64), sqlx::Error> {
        let posts = count_where(
            self,
            &format!(
                "SELECT COUNT(*) AS n FROM {} WHERE type = 'post' AND status = 'publish' AND authorId = ?",
                self.table("contents")
            ),
            uid,
        )
        .await?;
        let comments = count_where(
            self,
            &format!(
                "SELECT COUNT(*) AS n FROM {} WHERE status = 'approved' AND ownerId = ?",
                self.table("comments")
            ),
            uid,
        )
        .await?;
        let categories_sql = format!(
            "SELECT COUNT(*) AS n FROM {} WHERE type = 'category'",
            self.table("metas")
        );
        let categories_row = sqlx::query(&categories_sql)
            .fetch_optional(&self.pool)
            .await?;
        let categories = categories_row
            .as_ref()
            .map(|r| col_i64(r, "n").unwrap_or(0))
            .unwrap_or(0);
        let logged_sql = format!(
            "SELECT logged AS n FROM {} WHERE uid = ? LIMIT 1",
            self.table("users")
        );
        let row = sqlx::query(&logged_sql)
            .bind(uid)
            .fetch_optional(&self.pool)
            .await?;
        let logged = row
            .as_ref()
            .map(|r| col_i64(r, "n").unwrap_or(0))
            .unwrap_or(0);
        Ok((posts, comments, categories, logged))
    }

    pub async fn comment_by_coid(&self, coid: i64) -> Result<Option<Comment>, sqlx::Error> {
        let sql = format!(
            "SELECT coid, cid, created, author, authorId, ownerId, mail, url, ip, text, type, status, parent \
             FROM {} WHERE coid = ? LIMIT 1",
            self.table("comments")
        );
        let row = sqlx::query(&sql)
            .bind(coid)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.as_ref().map(map_comment_row))
    }

    pub async fn update_comment(
        &self,
        coid: i64,
        author: &str,
        mail: &str,
        url: &str,
        text: &str,
    ) -> Result<(), sqlx::Error> {
        let sql = format!(
            "UPDATE {} SET author = ?, mail = ?, url = ?, text = ? WHERE coid = ?",
            self.table("comments")
        );
        sqlx::query(&sql)
            .bind(trunc(author, 150))
            .bind(empty_null(&trunc(mail, 150)))
            .bind(empty_null(&trunc(url, 255)))
            .bind(text)
            .bind(coid)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn fields_for(&self, cid: i64) -> Result<Vec<FieldRow>, sqlx::Error> {
        let sql = format!(
            "SELECT name, type, str_value, int_value, CAST(float_value AS TEXT) AS float_text \
             FROM {} WHERE cid = ? ORDER BY name ASC",
            self.table("fields")
        );
        let rows = sqlx::query(&sql).bind(cid).fetch_all(&self.pool).await?;
        Ok(rows
            .iter()
            .map(|row| FieldRow {
                name: col_str(row, "name"),
                kind: col_str(row, "type"),
                str_value: col_str(row, "str_value"),
                int_value: col_i64(row, "int_value").unwrap_or(0),
                float_text: col_str(row, "float_text"),
            })
            .collect())
    }

    pub async fn replace_fields(&self, cid: i64, fields: &[FieldInput]) -> Result<(), sqlx::Error> {
        let sql = format!("DELETE FROM {} WHERE cid = ?", self.table("fields"));
        sqlx::query(&sql).bind(cid).execute(&self.pool).await?;
        let sql = format!(
            "INSERT INTO {} (cid, name, type, str_value, int_value, float_value) VALUES (?, ?, ?, ?, ?, ?)",
            self.table("fields")
        );
        for field in fields {
            if !field_name_ok(&field.name) {
                continue;
            }
            let kind = match field.kind.as_str() {
                "int" | "float" | "json" => field.kind.as_str(),
                _ => "str",
            };
            let (str_value, int_value, float_value) = match kind {
                "int" => (None, field.value.trim().parse::<i64>().unwrap_or(0), 0.0),
                "float" => (None, 0, field.value.trim().parse::<f64>().unwrap_or(0.0)),
                "json" => (
                    Some(serde_json::to_string(&field.value).unwrap_or_else(|_| "\"\"".into())),
                    0,
                    0.0,
                ),
                _ => (Some(field.value.clone()), 0, 0.0),
            };
            sqlx::query(&sql)
                .bind(cid)
                .bind(trunc(&field.name, 150))
                .bind(kind)
                .bind(str_value)
                .bind(int_value)
                .bind(float_value)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    pub async fn set_option(&self, name: &str, value: &str) -> Result<(), sqlx::Error> {
        let name = trunc(name, 32);
        let sql = format!(
            "UPDATE {} SET value = ? WHERE name = ? AND user = 0",
            self.table("options")
        );
        let res = sqlx::query(&sql)
            .bind(value)
            .bind(&name)
            .execute(&self.pool)
            .await?;
        if res.rows_affected() == 0 {
            let sql = format!(
                "INSERT INTO {} (name, user, value) VALUES (?, 0, ?)",
                self.table("options")
            );
            sqlx::query(&sql)
                .bind(&name)
                .bind(value)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    pub async fn save_user(&self, input: &UserInput) -> Result<i64, sqlx::Error> {
        let group_col = self.quote("group");
        let name = trunc(&input.name, 32);
        let screen = trunc(&input.screen, 32);
        let mail = trunc(&input.mail, 150);
        let url = trunc(&input.url, 150);
        let group = match input.group.as_str() {
            "administrator" | "editor" | "contributor" | "subscriber" => input.group.as_str(),
            _ => "subscriber",
        };
        if let Some(uid) = input.uid {
            if input.password.is_empty() {
                let sql = format!(
                    "UPDATE {} SET name = ?, mail = ?, url = ?, screenName = ?, {group_col} = ? WHERE uid = ?",
                    self.table("users")
                );
                sqlx::query(&sql)
                    .bind(&name)
                    .bind(&mail)
                    .bind(&url)
                    .bind(&screen)
                    .bind(group)
                    .bind(uid)
                    .execute(&self.pool)
                    .await?;
            } else {
                let sql = format!(
                    "UPDATE {} SET name = ?, password = ?, mail = ?, url = ?, screenName = ?, {group_col} = ? WHERE uid = ?",
                    self.table("users")
                );
                sqlx::query(&sql)
                    .bind(&name)
                    .bind(crate::auth::md5_hex(input.password.as_bytes()))
                    .bind(&mail)
                    .bind(&url)
                    .bind(&screen)
                    .bind(group)
                    .bind(uid)
                    .execute(&self.pool)
                    .await?;
            }
            Ok(uid)
        } else {
            let sql = format!(
                "INSERT INTO {} (name, password, mail, url, screenName, created, activated, logged, {group_col}, authCode) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, 0, ?, NULL)",
                self.table("users")
            );
            let res = sqlx::query(&sql)
                .bind(&name)
                .bind(crate::auth::md5_hex(input.password.as_bytes()))
                .bind(&mail)
                .bind(&url)
                .bind(&screen)
                .bind(input.created)
                .bind(input.created)
                .bind(group)
                .execute(&self.pool)
                .await?;
            last_id(self, res.last_insert_id().unwrap_or(0), "users", "uid").await
        }
    }

    pub async fn delete_user(&self, uid: i64) -> Result<(), sqlx::Error> {
        let sql = format!("DELETE FROM {} WHERE uid = ?", self.table("users"));
        sqlx::query(&sql).bind(uid).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn list_users(&self) -> Result<Vec<crate::auth::SessionUser>, sqlx::Error> {
        let group = self.quote("group");
        let sql = format!(
            "SELECT uid, name, mail, url, screenName, {group} AS user_group, logged FROM {} ORDER BY uid ASC",
            self.table("users")
        );
        let rows = sqlx::query(&sql).fetch_all(&self.pool).await?;
        Ok(rows
            .iter()
            .map(|row| crate::auth::SessionUser {
                uid: col_i64(row, "uid").unwrap_or(0),
                name: col_str(row, "name"),
                screen: col_str(row, "screenName"),
                mail: col_str(row, "mail"),
                url: col_str(row, "url"),
                group: col_str(row, "user_group"),
                logged: col_i64(row, "logged").unwrap_or(0),
            })
            .collect())
    }

    pub async fn meta_by_name(&self, kind: &str, name: &str) -> Result<Option<i64>, sqlx::Error> {
        let sql = format!(
            "SELECT mid FROM {} WHERE type = ? AND name = ? LIMIT 1",
            self.table("metas")
        );
        let row = sqlx::query(&sql)
            .bind(kind)
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.and_then(|r| col_i64(&r, "mid").ok()))
    }

    pub async fn count_admins(&self) -> Result<i64, sqlx::Error> {
        let group = self.quote("group");
        let sql = format!(
            "SELECT COUNT(*) AS n FROM {} WHERE {group} = 'administrator'",
            self.table("users")
        );
        let row = sqlx::query(&sql).fetch_optional(&self.pool).await?;
        Ok(row
            .as_ref()
            .map(|r| col_i64(r, "n").unwrap_or(0))
            .unwrap_or(0))
    }
}

pub fn attachment_text(name: &str, path: &str, size: i64, ext: &str, mime: &str) -> String {
    crate::phpval::serialize_string_map(&[
        ("name", PhpValue::String(name.into())),
        ("path", PhpValue::String(path.into())),
        ("size", PhpValue::Int(size)),
        ("type", PhpValue::String(ext.into())),
        ("mime", PhpValue::String(mime.into())),
    ])
}

pub fn attachment_fields(text: &str) -> Option<(String, String, String)> {
    let v = crate::phpval::parse_php(text).ok()?;
    let name = v.get_key("name")?.to_string_value()?;
    let path = v.get_key("path")?.to_string_value()?;
    let mime = v
        .get_key("mime")
        .and_then(|m| m.to_string_value())
        .unwrap_or_else(|| "application/octet-stream".into());
    Some((name, path, mime))
}

pub fn attachment_markup(base: &str, text: &str) -> String {
    let Some((name, path, _mime)) = attachment_fields(text) else {
        return "<p>附件信息不完整</p>".into();
    };
    let url = format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let safe_url = crate::text::html_escape(&url);
    let safe_name = crate::text::html_escape(&name);
    let images = ["jpg", "jpeg", "gif", "png", "tiff", "bmp", "webp", "avif"];
    if images.contains(&ext.as_str()) {
        format!("<p><img src=\"{safe_url}\" alt=\"{safe_name}\"></p><p><a href=\"?download=1\">{safe_name}</a></p>")
    } else {
        format!("<p><a href=\"{safe_url}\">{safe_name}</a> <a href=\"?download=1\">下载</a></p>")
    }
}

fn normalize_status(status: &str) -> &str {
    match status {
        "publish" | "hidden" | "private" | "waiting" | "draft" => status,
        _ => "draft",
    }
}

fn trunc(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn empty_null(s: &str) -> Option<&str> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn clean_slug(s: &str) -> String {
    s.trim()
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '/' | '?' | '#' | '&'))
        .take(140)
        .collect()
}

fn slugify(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if (c == '-' || c == '_') && !out.ends_with('-') {
            out.push('-');
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_matches('-').chars().take(120).collect()
}

async fn last_id(db: &Db, id: i64, table: &str, col: &str) -> Result<i64, sqlx::Error> {
    if id > 0 {
        return Ok(id);
    }
    let sql = format!(
        "SELECT MAX({col}) AS n FROM {}",
        db.table(table)
    );
    let row = sqlx::query(&sql).fetch_optional(&db.pool).await?;
    Ok(row
        .as_ref()
        .map(|r| col_i64(r, "n").unwrap_or(0))
        .unwrap_or(0))
}

async fn unique_slug(db: &Db, cid: i64, wanted: &str, title: &str) -> Result<String, sqlx::Error> {
    let base = {
        let raw = if wanted.trim().is_empty() {
            slugify(title)
        } else {
            clean_slug(wanted)
        };
        if raw.is_empty() {
            cid.to_string()
        } else {
            raw
        }
    };
    let mut candidate = trunc(&base, 140);
    for n in 0..20 {
        if n > 0 {
            candidate = trunc(&format!("{base}-{cid}"), 150);
        }
        let sql = format!(
            "SELECT cid FROM {} WHERE slug = ? AND cid <> ? LIMIT 1",
            db.table("contents")
        );
        let row = sqlx::query(&sql)
            .bind(&candidate)
            .bind(cid)
            .fetch_optional(&db.pool)
            .await?;
        if row.is_none() {
            return Ok(candidate);
        }
    }
    Ok(cid.to_string())
}

async fn replace_terms(
    db: &Db,
    cid: i64,
    categories: &[i64],
    tags: &[String],
) -> Result<(), sqlx::Error> {
    let old = related_mids(db, cid).await?;
    let sql = format!("DELETE FROM {} WHERE cid = ?", db.table("relationships"));
    sqlx::query(&sql).bind(cid).execute(&db.pool).await?;
    let mut mids = categories.to_vec();
    for name in tags {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let mid = if let Some(mid) = db.meta_by_name("tag", name).await? {
            mid
        } else {
            db.save_meta(&MetaInput {
                mid: None,
                name: name.to_string(),
                slug: String::new(),
                kind: "tag".into(),
                description: String::new(),
                parent: 0,
            })
            .await?
        };
        if !mids.contains(&mid) {
            mids.push(mid);
        }
    }
    for mid in &mids {
        let sql = format!(
            "INSERT INTO {} (cid, mid) VALUES (?, ?)",
            db.table("relationships")
        );
        let _ = sqlx::query(&sql).bind(cid).bind(mid).execute(&db.pool).await;
    }
    let mut touch = old;
    for mid in mids {
        if !touch.contains(&mid) {
            touch.push(mid);
        }
    }
    for mid in touch {
        recount_meta(db, mid).await?;
    }
    Ok(())
}

async fn related_mids(db: &Db, cid: i64) -> Result<Vec<i64>, sqlx::Error> {
    let sql = format!(
        "SELECT mid FROM {} WHERE cid = ?",
        db.table("relationships")
    );
    let rows = sqlx::query(&sql).bind(cid).fetch_all(&db.pool).await?;
    Ok(rows
        .iter()
        .filter_map(|r| col_i64(r, "mid").ok())
        .collect())
}

async fn recount_meta(db: &Db, mid: i64) -> Result<(), sqlx::Error> {
    let sql = format!(
        "SELECT COUNT(*) AS n FROM {rel} r JOIN {contents} c ON r.cid = c.cid \
         WHERE r.mid = ? AND c.type = 'post' AND c.status = 'publish'",
        rel = db.table("relationships"),
        contents = db.table("contents"),
    );
    let row = sqlx::query(&sql).bind(mid).fetch_optional(&db.pool).await?;
    let n = row
        .as_ref()
        .map(|r| col_i64(r, "n").unwrap_or(0))
        .unwrap_or(0);
    let sql = format!("UPDATE {} SET count = ? WHERE mid = ?", db.table("metas"));
    sqlx::query(&sql)
        .bind(n)
        .bind(mid)
        .execute(&db.pool)
        .await?;
    Ok(())
}

async fn comment_cid(db: &Db, coid: i64) -> Result<Option<i64>, sqlx::Error> {
    let sql = format!(
        "SELECT cid FROM {} WHERE coid = ? LIMIT 1",
        db.table("comments")
    );
    let row = sqlx::query(&sql).bind(coid).fetch_optional(&db.pool).await?;
    Ok(row.and_then(|r| col_i64(&r, "cid").ok()))
}

async fn refresh_comment_count(db: &Db, cid: i64) -> Result<(), sqlx::Error> {
    let sql = format!(
        "SELECT COUNT(*) AS n FROM {} WHERE status = 'approved' AND cid = ?",
        db.table("comments")
    );
    let row = sqlx::query(&sql).bind(cid).fetch_optional(&db.pool).await?;
    let n = row
        .as_ref()
        .map(|r| col_i64(r, "n").unwrap_or(0))
        .unwrap_or(0);
    let sql = format!(
        "UPDATE {} SET commentsNum = ? WHERE cid = ?",
        db.table("contents")
    );
    sqlx::query(&sql)
        .bind(n)
        .bind(cid)
        .execute(&db.pool)
        .await?;
    Ok(())
}

async fn scalar(db: &Db, sql: &str, cid: i64, extra: Option<&str>) -> Result<i64, sqlx::Error> {
    let mut q = sqlx::query(sql).bind(cid);
    if let Some(extra) = extra {
        q = q.bind(extra);
    }
    let row = q.fetch_optional(&db.pool).await?;
    Ok(row
        .as_ref()
        .map(|r| col_i64(r, "n").unwrap_or(0))
        .unwrap_or(0))
}

fn content_cols(db: &Db) -> String {
    let order = db.order_col();
    format!(
        "c.cid, c.title, c.slug, c.created, c.modified, c.text, c.{order} AS sort_order, \
         c.authorId, c.template, c.type, c.status, c.password, c.commentsNum, \
         c.allowComment, c.allowPing, c.allowFeed, c.parent, \
         u.screenName AS screen_name, u.name AS user_name, u.mail AS user_mail, u.url AS user_url"
    )
}

fn map_post_row(row: &sqlx::any::AnyRow) -> Result<Post, sqlx::Error> {
    use std::collections::HashMap;
    Ok(Post {
        cid: col_i64(row, "cid")?,
        title: col_str(row, "title"),
        slug: col_str(row, "slug"),
        created: col_i64(row, "created")?,
        modified: col_i64(row, "modified")?,
        text: col_str(row, "text"),
        sort_order: col_i64(row, "sort_order")?,
        author_id: col_i64(row, "authorId")?,
        template: col_str(row, "template"),
        kind: col_str(row, "type"),
        status: col_str(row, "status"),
        password: col_str(row, "password"),
        comments_num: col_i64(row, "commentsNum")?,
        allow_comment: is_yes(&col_str(row, "allowComment")),
        allow_ping: is_yes(&col_str(row, "allowPing")),
        allow_feed: is_yes(&col_str(row, "allowFeed")),
        parent: col_i64(row, "parent")?,
        author_screen: col_str(row, "screen_name"),
        author_name: col_str(row, "user_name"),
        author_mail: col_str(row, "user_mail"),
        author_url: col_str(row, "user_url"),
        categories: Vec::new(),
        tags: Vec::new(),
        fields: HashMap::new(),
    })
}

fn map_meta_row(row: &sqlx::any::AnyRow) -> Result<Meta, sqlx::Error> {
    Ok(Meta {
        mid: col_i64(row, "mid")?,
        name: col_str(row, "name"),
        slug: col_str(row, "slug"),
        kind: col_str(row, "type"),
        description: col_str(row, "description"),
        count: col_i64(row, "count")?,
        sort_order: col_i64(row, "sort_order")?,
        parent: col_i64(row, "parent")?,
    })
}

fn map_comment_row(row: &sqlx::any::AnyRow) -> Comment {
    Comment {
        coid: col_i64(row, "coid").unwrap_or(0),
        cid: col_i64(row, "cid").unwrap_or(0),
        created: col_i64(row, "created").unwrap_or(0),
        author: col_str(row, "author"),
        author_id: col_i64(row, "authorId").unwrap_or(0),
        owner_id: col_i64(row, "ownerId").unwrap_or(0),
        mail: col_str(row, "mail"),
        url: col_str(row, "url"),
        ip: col_str(row, "ip"),
        text: col_str(row, "text"),
        kind: col_str(row, "type"),
        status: col_str(row, "status"),
        parent: col_i64(row, "parent").unwrap_or(0),
    }
}

/// 撰写页「引用通告」文本框：一行一个 http(s) 地址，去掉空行和重复。
/// 原版在发布时向这些地址发 HTTP ping。这里只解析，不发起请求。
pub fn trackback_targets(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in raw.split(['\r', '\n']) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !(line.starts_with("http://") || line.starts_with("https://")) {
            continue;
        }
        if out.iter().any(|seen| seen == line) {
            continue;
        }
        out.push(line.to_string());
    }
    out
}

pub fn field_name_ok(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

async fn count_where(db: &Db, sql: &str, uid: i64) -> Result<i64, sqlx::Error> {
    let row = sqlx::query(sql).bind(uid).fetch_optional(&db.pool).await?;
    Ok(row
        .as_ref()
        .map(|r| col_i64(r, "n").unwrap_or(0))
        .unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::trackback_targets;

    #[test]
    fn trackback_keeps_unique_http_lines() {
        let raw = "\nhttps://a.example/tb\nftp://skip\nhttps://a.example/tb\nhttp://b.example\n";
        assert_eq!(
            trackback_targets(raw),
            vec![
                "https://a.example/tb".to_string(),
                "http://b.example".to_string()
            ]
        );
    }
}

async fn attach_terms(db: &Db, posts: &mut [Post]) -> Result<(), sqlx::Error> {
    if posts.is_empty() {
        return Ok(());
    }
    let marks = vec!["?"; posts.len()].join(",");
    let order = db.order_col();
    let sql = format!(
        "SELECT r.cid AS rel_cid, m.mid, m.name, m.slug, m.type, m.description, m.count, \
         m.{order} AS sort_order, m.parent FROM {rel} r JOIN {metas} m ON r.mid = m.mid \
         WHERE r.cid IN ({marks})",
        rel = db.table("relationships"),
        metas = db.table("metas"),
    );
    let mut q = sqlx::query(&sql);
    for p in posts.iter() {
        q = q.bind(p.cid);
    }
    let rows = q.fetch_all(&db.pool).await?;
    for row in &rows {
        let cid = col_i64(row, "rel_cid").unwrap_or(0);
        let meta = map_meta_row(row)?;
        if let Some(post) = posts.iter_mut().find(|p| p.cid == cid) {
            if meta.kind == "category" {
                post.categories.push(meta);
            } else if meta.kind == "tag" {
                post.tags.push(meta);
            }
        }
    }
    Ok(())
}
