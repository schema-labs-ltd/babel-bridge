//! Image and album payload and route tests.

use super::*;

#[test]
fn image_payload_validation_accepts_valid_png() {
    let png_base64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=";
    let payload =
        validate_image_payload(png_base64, "image/png; charset=binary").expect("valid png");

    assert_eq!(payload.mime_type, "image/png");
    assert!(payload.decoded_size > 0);
}

#[test]
fn album_payload_validation_preserves_order_and_requires_multiple_images() {
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=";
    let images = vec![
        SendImageItemRequest {
            media_data: png.to_string(),
            mime_type: "image/png".to_string(),
        },
        SendImageItemRequest {
            media_data: png.to_string(),
            mime_type: "image/png".to_string(),
        },
    ];
    let validated = validate_album_payloads(&images).expect("valid album");
    assert_eq!(validated.len(), 2);
    assert_eq!(validated[0].mime_type, "image/png");

    let error = validate_album_payloads(&images[..1]).unwrap_err();
    assert_eq!(error, "Choose between 2 and 30 images for an album.");
}

#[tokio::test]
async fn send_image_route_accepts_payloads_larger_than_axum_default_body_limit() {
    let (state, data_dir) = test_state(None);
    let mut jpeg = vec![0_u8; 2 * 1024 * 1024];
    jpeg[..3].copy_from_slice(&[0xff, 0xd8, 0xff]);
    let request = HttpRequest::builder()
        .method("POST")
        .uri("/api/send-image")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "contactId": "chat@example.test",
                "mediaData": BASE64_STANDARD.encode(jpeg),
                "mimeType": "image/jpeg"
            })
            .to_string(),
        ))
        .expect("request");

    let response = create_router(state)
        .oneshot(request)
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn send_images_route_accepts_album_payload_larger_than_axum_default_body_limit() {
    let (state, data_dir) = test_state(None);
    let mut jpeg = vec![0_u8; 1024 * 1024 + 256];
    jpeg[..3].copy_from_slice(&[0xff, 0xd8, 0xff]);
    let encoded = BASE64_STANDARD.encode(jpeg);
    let request = HttpRequest::builder()
        .method("POST")
        .uri("/api/send-images")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
                "contactId": "chat@example.test",
                "images": [
                    { "mediaData": encoded, "mimeType": "image/jpeg" },
                    { "mediaData": encoded, "mimeType": "image/jpeg" }
                ]
            })
            .to_string(),
        ))
        .expect("request");

    let response = create_router(state)
        .oneshot(request)
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[tokio::test]
async fn staged_album_route_checkpoints_each_photo_before_send() {
    let (state, data_dir) = test_state(None);
    let router = create_router(state.clone());
    let create = HttpRequest::builder()
        .method("POST")
        .uri("/api/photo-albums")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!({
            "jobId": "job-0012", "contactId": "chat@example.test", "photoCount": 2
            })
            .to_string(),
        ))
        .unwrap();
    assert_eq!(
        router.clone().oneshot(create).await.unwrap().status(),
        StatusCode::OK
    );

    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=";
    for index in 0..2 {
        let stage = HttpRequest::builder()
            .method("PUT")
            .uri(format!("/api/photo-albums/job-0012/images/{index}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "mediaData": png, "mimeType": "image/png"
                })
                .to_string(),
            ))
            .unwrap();
        assert_eq!(
            router.clone().oneshot(stage).await.unwrap().status(),
            StatusCode::OK
        );
    }
    let albums = state.pending_photo_albums.read().await;
    let album = albums.get("job-0012").expect("staged album");
    assert!(album.images.iter().all(Option::is_some));
    drop(albums);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn large_album_confirmation_budget_exceeds_previous_three_minute_limit() {
    assert!(album_send_result_timeout(10) >= Duration::from_secs(10 * 60));
    assert!(album_send_result_timeout(2) >= ALBUM_SEND_RESULT_TIMEOUT);
}

#[test]
fn image_payload_validation_rejects_invalid_base64() {
    let error = validate_image_payload("not valid base64", "image/png").unwrap_err();
    assert_eq!(error, "media_data must be valid base64");
}

#[test]
fn image_payload_validation_rejects_mime_mismatch() {
    let png_base64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=";
    let error = validate_image_payload(png_base64, "image/jpeg").unwrap_err();

    assert_eq!(error, "media_data does not match MIME type image/jpeg");
}

#[test]
fn image_payload_validation_rejects_data_url_prefix() {
    let error =
        validate_image_payload("data:image/png;base64,iVBORw0KGgo=", "image/png").unwrap_err();

    assert_eq!(
        error,
        "media_data must be raw base64 without a data URL prefix"
    );
}

#[test]
fn image_payload_validation_rejects_oversized_encoded_payload() {
    let oversized = "A".repeat(MAX_IMAGE_BASE64_BYTES + 1);
    let error = validate_image_payload(&oversized, "image/png").unwrap_err();

    assert_eq!(error, "Image is too large. Maximum decoded size is 16MB.");
}
