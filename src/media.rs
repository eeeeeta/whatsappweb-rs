extern crate base64;
extern crate json;
extern crate image;

use std::io::Cursor;

use image::{GenericImageView, RGB};
use image::jpeg::JPEGEncoder;
use reqwest;

use crate::MediaType;
use crate::crypto;
use crate::message::FileInfo;
use crate::errors::*;

const USER_AGENT: &'static str = concat!("ww-rs-eta/", env!("CARGO_PKG_VERSION"));

pub fn generate_thumbnail_and_get_size(image: &[u8]) -> (Vec<u8>, (u32, u32)) {
    let image = image::load_from_memory(image).unwrap();

    let size = (image.height(), image.width());
    let thumbnail = image.thumbnail(160, 160).to_rgb();

    let mut thumbnail_writter = Cursor::new(Vec::new());

    JPEGEncoder::new(&mut thumbnail_writter).encode(&thumbnail, thumbnail.width(), thumbnail.height(), RGB(8)).unwrap();

    (thumbnail_writter.into_inner(), size)
}

/// Download file from servers and decrypt it
pub async fn download_file(file_info: FileInfo, media_type: MediaType) -> Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()?;
    let response = client
        .get(&file_info.url)
        .send()
        .await?;

    if response.status() != reqwest::StatusCode::from_u16(200).unwrap() {
        error!("{:?}", response);
        return Err(WaError::Untyped("Non 200 status received"));
    }
    let cyphertext = response.bytes().await?;

    crypto::decrypt_media_message(&file_info.key, media_type, &cyphertext)
}

fn path_for(media_type: MediaType) -> [&'static str; 2] {
    match media_type {
        MediaType::Image => ["mms", "image"],
        MediaType::Video => ["mms", "video"],
        MediaType::Document => ["mms", "document"],
        MediaType::Audio => ["mms", "audio"],
    }
}

/// Upload file to servers and encrypt it
pub async fn upload_file(file: &[u8], mime: String, media_type: MediaType, auth: &str, host: &str) -> Result<FileInfo> {
     let sha256 = crypto::sha256(file);

    let (file_encrypted, key) = crypto::encrypt_media_message(media_type, file);
    let enc_sha256 = crypto::sha256(&file_encrypted);
    let token = base64::encode_config(&enc_sha256, base64::URL_SAFE_NO_PAD);
    let size = file.len();
    let mut url = reqwest::Url::parse(&format!("https://{}/", host)).unwrap();
    url.path_segments_mut().unwrap()
        .extend(&path_for(media_type))
        .push(&token);

    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()?;

    let response = client
        .post(&url.to_string())
        .query(&[
            ("auth", auth),
            ("token", &token),
        ])
        .header("Origin", "https://web.whatsapp.com")
        .header("Referer", "https://web.whatsapp.com/")
        .body(file_encrypted)
        .send().await?;


    if response.status() != reqwest::StatusCode::from_u16(200).unwrap() {
        error!("{:?}", response);
        error!("{:?}", response.text().await?);
        return Err(WaError::Untyped("Non 200 status received"));
    }

    let text = response.text().await?;
    let json = json::parse(&text)?;

    Ok(FileInfo {
        url: json["url"].as_str().ok_or(WaError::JsonFieldMissing("url"))?.into(),
        mime,
        sha256,
        enc_sha256,
        size,
        key,
    })
}
