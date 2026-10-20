use axum::{
    Router,
    body::Body,
    http::{Request, header},
    middleware,
};
use serde_json::json;
use std::path::Path;
use tower::ServiceExt;

pub async fn authenticated(
    router: Router,
    directory: &Path,
) -> Result<Router, Box<dyn std::error::Error>> {
    let path = directory.join("bootstrap-admin.json");
    let temporary = if path.exists() {
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
        Some(
            value["password"]
                .as_str()
                .ok_or("Missing fixture password")?
                .to_owned(),
        )
    } else {
        None
    };
    let login = |password: String| {
        let router = router.clone();
        async move {
            let response = router
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/auth/login")
                        .header("Host", "localhost:9400")
                        .header("Content-Type", "application/json")
                        .body(Body::from(serde_json::to_vec(
                            &json!({"username":"admin","password":password}),
                        )?))?,
                )
                .await?;
            if !response.status().is_success() {
                return Err("Fixture login failed".into());
            }
            let cookie = response
                .headers()
                .get(header::SET_COOKIE)
                .ok_or("Missing fixture cookie")?
                .to_str()?
                .split(';')
                .next()
                .ok_or("Missing fixture cookie")?
                .to_owned();
            Ok::<_, Box<dyn std::error::Error>>(cookie)
        }
    };
    let password = "synthetic-fixture-password-123";
    if let Some(temporary) = temporary {
        let cookie = login(temporary.clone()).await?;
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/password")
                    .header("Host", "localhost:9400")
                    .header("Content-Type", "application/json")
                    .header(header::COOKIE, cookie)
                    .body(Body::from(serde_json::to_vec(
                        &json!({"current_password":temporary,"password":password}),
                    )?))?,
            )
            .await?;
        if !response.status().is_success() {
            return Err("Fixture password change failed".into());
        }
    }
    let cookie: axum::http::HeaderValue = login(password.into()).await?.parse()?;
    Ok(router.layer(middleware::from_fn(
        move |mut request: Request<Body>, next: middleware::Next| {
            let cookie = cookie.clone();
            async move {
                if !request.headers().contains_key(header::COOKIE) {
                    request.headers_mut().insert(header::COOKIE, cookie);
                }
                next.run(request).await
            }
        },
    )))
}
