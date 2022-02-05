use std::{sync::Arc, time::Duration};

use anyhow::anyhow;
use askama::Template;
use axum::{
    extract::{
        ws::{Message, WebSocket},
        Extension, Path, WebSocketUpgrade,
    },
    http::StatusCode,
    response::{Headers, Html, IntoResponse},
};
use headers::HeaderMap;
use tokio::select;

use crate::{
    app_model::{Context, DynContext},
    membership_model::Membership,
    GIT_HASH,
};

pub async fn ws_upgrade(
    Extension(ctx): Extension<DynContext>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(ctx, socket))
}

async fn handle_socket(ctx: Arc<Context>, mut socket: WebSocket) {
    let mut rx = ctx.vistor_rx.clone();
    let mut interval = tokio::time::interval(Duration::from_secs(8));

    loop {
        select! {
            Ok(()) = rx.changed() => {
                let msg = rx.borrow().to_string();
                let res = socket.send(Message::Text(msg.clone())).await;
                if res.is_err() {
                    break;
                }
            }
            _ = interval.tick() => {
                let res = socket.send(Message::Ping(vec![])).await;
                if res.is_err() {
                    break;
                }
            }
        }
    }
}

pub async fn show_badge(
    Path(domain): Path<String>,
    headers: HeaderMap,
    Extension(ctx): Extension<DynContext>,
) -> impl IntoResponse {
    let mut v_type = crate::app_model::VistorType::Badge;

    let domain_referrer = get_domain_from_headers(&headers);
    if domain_referrer.is_err() || domain_referrer.unwrap().ne(&domain) {
        v_type = crate::app_model::VistorType::ICON;
    }

    let tend = ctx.boring_vistor(v_type, &domain, &headers).await;
    if tend.is_err() {
        return (
            StatusCode::NOT_FOUND,
            Headers([("content-type", "text/plain")]),
            tend.err().unwrap().to_string(),
        );
    }

    let headers = Headers([("content-type", "image/svg+xml")]);
    let len: usize = 10;
    let read = ctx.badge_render_cache.read().await;
    let cache = read.get(&len);
    let content = if let Some(v) = cache {
        v.clone()
    } else {
        drop(read);
        let v = ctx.badge.render_svg(tend.unwrap() as usize);
        let mut write = ctx.badge_render_cache.write().await;
        write.insert(len, v.clone());
        v
    };
    (StatusCode::OK, headers, content)
}

pub async fn show_favicon(
    Path(domain): Path<String>,
    headers: HeaderMap,
    Extension(ctx): Extension<DynContext>,
) -> impl IntoResponse {
    let tend = ctx
        .boring_vistor(crate::app_model::VistorType::ICON, &domain, &headers)
        .await;
    if tend.is_err() {
        return (
            StatusCode::NOT_FOUND,
            Headers([("content-type", "text/plain")]),
            tend.err().unwrap().to_string(),
        );
    }
    let headers = Headers([("content-type", "image/svg+xml")]);
    let len: usize = 10;
    let read = ctx.favicon_render_cache.read().await;
    let cache = read.get(&len);
    let content = if let Some(v) = cache {
        v.clone()
    } else {
        drop(read);
        let v = ctx.favicon.render_svg(tend.unwrap() as usize);
        let mut write = ctx.favicon_render_cache.write().await;
        write.insert(len, v.clone());
        v
    };
    (StatusCode::OK, headers, content)
}

pub async fn show_icon(
    Path(domain): Path<String>,
    headers: HeaderMap,
    Extension(ctx): Extension<DynContext>,
) -> impl IntoResponse {
    let tend = ctx
        .boring_vistor(crate::app_model::VistorType::ICON, &domain, &headers)
        .await;
    if tend.is_err() {
        return (
            StatusCode::NOT_FOUND,
            Headers([("content-type", "text/plain")]),
            tend.err().unwrap().to_string(),
        );
    }
    let headers = Headers([("content-type", "image/svg+xml")]);
    let len: usize = 10;
    let read = ctx.icon_render_cache.read().await;
    let cache = read.get(&len);
    let content = if let Some(v) = cache {
        v.clone()
    } else {
        drop(read);
        let v = ctx.icon.render_svg(tend.unwrap() as usize);
        let mut write = ctx.icon_render_cache.write().await;
        write.insert(len, v.clone());
        v
    };
    (StatusCode::OK, headers, content)
}

#[derive(Template)]
#[template(path = "index.html")]
struct HomeTemplate {
    version: String,
    membership: Vec<Membership>,
}

pub async fn home_page(
    Extension(ctx): Extension<DynContext>,
    headers: HeaderMap,
) -> Result<Html<String>, String> {
    let domain = get_domain_from_headers(&headers);
    if domain.is_ok() {
        let _ = ctx
            .boring_vistor(
                crate::app_model::VistorType::Referrer,
                &domain.unwrap(),
                &headers,
            )
            .await;
    }
    let referrer_read = ctx.referrer.read().await;
    let pv_read = ctx.page_view.read().await;

    let mut rank_vec: Vec<(i64, i64)> = Vec::new();

    for k in ctx.id2member.keys() {
        rank_vec.push((
            k.to_owned(),
            referrer_read.get(k).unwrap_or(&0).to_owned() * 5
                + pv_read.get(k).unwrap_or(&0).to_owned(),
        ));
    }

    rank_vec.sort_by(|a, b| b.1.cmp(&a.1));

    let mut membership = Vec::new();
    for v in rank_vec {
        membership.push(ctx.id2member.get(&v.0).unwrap().to_owned());
    }

    let tpl = HomeTemplate {
        membership,
        version: GIT_HASH[0..8].to_string(),
    };
    let html = tpl.render().map_err(|err| err.to_string())?;
    Ok(Html(html))
}

#[derive(Template)]
#[template(path = "join_us.html")]
struct JoinUsTemplate {
    version: String,
}

pub async fn join_us_page() -> Result<Html<String>, String> {
    let tpl = JoinUsTemplate {
        version: GIT_HASH[0..8].to_string(),
    };
    let html = tpl.render().map_err(|err| err.to_string())?;
    Ok(Html(html))
}

fn get_domain_from_headers(headers: &HeaderMap) -> Result<String, anyhow::Error> {
    let referrer_header = headers.get("Referer");
    if referrer_header.is_none() {
        return Err(anyhow!("no referrer header"));
    }

    let referrer_str = String::from_utf8(referrer_header.unwrap().as_bytes().to_vec());
    if referrer_str.is_err() {
        return Err(anyhow!("referrer header is not valid utf-8 string"));
    }

    let referrer_url = url::Url::parse(&referrer_str.unwrap());
    if referrer_url.is_err() {
        return Err(anyhow!("referrer header is not valid URL"));
    }

    let referrer_url = referrer_url.unwrap();
    if referrer_url.domain().is_none() {
        return Err(anyhow!("referrer header doesn't contains a valid domain"));
    }

    return Ok(referrer_url.domain().unwrap().to_string());
}
