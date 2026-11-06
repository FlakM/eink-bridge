use super::*;

/// A reader over a buffered multipart body. Errors are the 400 message.
fn reader(headers: &HeaderMap, body: Bytes) -> Result<MulterMultipart<'static>, String> {
    let boundary = parse_boundary(content_type(headers))
        .map_err(|error| format!("invalid multipart boundary: {error}"))?;
    let stream = stream::once(async move { Ok::<Bytes, std::io::Error>(body) });
    Ok(MulterMultipart::new(stream, boundary))
}

async fn next_field<'a>(
    multipart: &mut MulterMultipart<'a>,
) -> Result<Option<multer::Field<'a>>, String> {
    multipart
        .next_field()
        .await
        .map_err(|error| format!("failed to parse multipart body: {error}"))
}

fn field_error(field: &str, error: multer::Error) -> String {
    format!("failed to read {field}: {error}")
}

pub(super) async fn parse_multipart_submit(
    headers: &HeaderMap,
    body: Bytes,
) -> Result<Submission, SubmitError> {
    let bad = |message: String| (StatusCode::BAD_REQUEST, message);
    let mut multipart = reader(headers, body).map_err(bad)?;
    let mut typed_notes = String::new();
    let mut images = Vec::new();
    let mut stroke_data: Option<serde_json::Value> = None;
    let mut annotations: Vec<AnnotationGroup> = Vec::new();
    let mut verdict_text: Option<String> = None;
    let mut document_version: Option<u32> = None;

    let field_error = |field: &str, error| bad(field_error(field, error));
    while let Some(field) = next_field(&mut multipart).await.map_err(bad)? {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "typed_notes" => {
                typed_notes = field
                    .text()
                    .await
                    .map_err(|e| field_error("typed_notes", e))?;
            }
            "annotation" => {
                let data = field
                    .bytes()
                    .await
                    .map_err(|e| field_error("annotation", e))?;
                if !data.starts_with(PNG_MAGIC) {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        "annotation is not a PNG image".into(),
                    ));
                }
                images.push(data);
            }
            "stroke_data" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| field_error("stroke_data", e))?;
                if !text.is_empty() {
                    stroke_data = serde_json::from_str(&text).ok();
                }
            }
            "annotations" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| field_error("annotations", e))?;
                if !text.is_empty() {
                    annotations = serde_json::from_str(&text).map_err(|error| {
                        (
                            StatusCode::BAD_REQUEST,
                            format!("invalid annotations JSON: {error}"),
                        )
                    })?;
                }
            }
            // The JSON path already accepts an explicit verdict; a multipart post that
            // carries one must not have it silently dropped, and an unusable tag must be
            // reported rather than ignored.
            "verdict" => {
                verdict_text = Some(field.text().await.map_err(|e| field_error("verdict", e))?);
            }
            "document_version" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| field_error("document_version", e))?;
                document_version = Some(text.trim().parse().map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("invalid document_version: {text}"),
                    )
                })?);
            }
            _ => {}
        }
    }

    let verdict = parse_verdict_override(verdict_text)?;
    Ok(Submission {
        typed_notes,
        images,
        verdict,
        stroke_data,
        annotations,
        document_version,
    })
}

/// A create request may carry the images its document displays, so the very first
/// version of the page shows them instead of rendering broken and reloading a second
/// later. Fields are the same as the JSON request; `image` repeats once per file, and the
/// part's file name is the name the markdown references as `image/<name>`.
pub(super) async fn parse_multipart_create(
    headers: &HeaderMap,
    body: Bytes,
) -> Result<(CreateSessionRequest, Vec<crate::session::SessionImage>), Response> {
    let mut multipart = reader(headers, body).map_err(bad_request)?;
    let mut request = CreateSessionRequest {
        schema_version: Some(SCHEMA_VERSION),
        title: None,
        content: String::new(),
        callback_url: None,
        tags: HashMap::new(),
        starred: false,
        origin: None,
    };
    let mut images = Vec::new();
    let mut has_content = false;

    let field_error = |field: &str, error| bad_request(field_error(field, error));
    while let Some(field) = next_field(&mut multipart).await.map_err(bad_request)? {
        match field.name().unwrap_or("") {
            "content" => {
                request.content = field.text().await.map_err(|e| field_error("content", e))?;
                has_content = true;
            }
            "title" => {
                let text = field.text().await.map_err(|e| field_error("title", e))?;
                request.title = (!text.trim().is_empty()).then_some(text);
            }
            "callback_url" => {
                request.callback_url = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| field_error("callback_url", e))?,
                );
            }
            "starred" => {
                let text = field.text().await.map_err(|e| field_error("starred", e))?;
                request.starred = matches!(text.trim(), "true" | "1");
            }
            "tags" => {
                let text = field.text().await.map_err(|e| field_error("tags", e))?;
                if !text.trim().is_empty() {
                    request.tags = serde_json::from_str(&text)
                        .map_err(|e| bad_request(format!("invalid tags JSON: {e}")))?;
                }
            }
            "origin" => {
                let text = field.text().await.map_err(|e| field_error("origin", e))?;
                if !text.trim().is_empty() {
                    request.origin = Some(
                        serde_json::from_str(&text)
                            .map_err(|e| bad_request(format!("invalid origin JSON: {e}")))?,
                    );
                }
            }
            "image" => {
                // A browser sends the whole client path; only the file name is used.
                let name = field
                    .file_name()
                    .unwrap_or_default()
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or_default()
                    .to_string();
                let Some(content_type) = document_image_type(&name) else {
                    return Err(bad_request(format!(
                        "image name {name:?} must be a single file name ending in .png, .jpg, .jpeg, .webp or .gif"
                    )));
                };
                let data = field.bytes().await.map_err(|e| field_error("image", e))?;
                if !bytes_are_image(content_type, &data) {
                    return Err(bad_request(format!(
                        "the bytes of {name} are not a {} image",
                        format_label(content_type)
                    )));
                }
                images.push(crate::session::SessionImage {
                    name,
                    bytes: data.to_vec(),
                });
            }
            _ => {}
        }
    }
    if !has_content {
        return Err(bad_request("a create request needs a content field".into()));
    }
    Ok((request, images))
}
