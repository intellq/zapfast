//! Link previews for text we send: the title, description, and picture a web
//! page gives for itself (its Open Graph tags, or its title and description),
//! fetched while the link is in the composer and carried in the message, as
//! the phone and WhatsApp's own clients do.
//!
//! The page is read from this computer, through the proxy in Settings, so the
//! linked site sees the request, as it does when the phone builds a preview.

use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::mpsc;
use ureq::ResponseExt;
use whatsapp_rust::download::MediaType;
use whatsapp_rust::prelude::{Client, MessageField, wa};
use whatsapp_rust::upload::UploadOptions;

use super::{Command, encode_jpeg};
use crate::model::{LinkCard, LinkImage, LinkPreview};

/// How long typing must pause on a link before its page is fetched: each key
/// in a link typed by hand asks for a new one.
const PAUSE: Duration = Duration::from_millis(500);
/// The whole of a request, answer included.
const TIMEOUT: Duration = Duration::from_secs(8);
/// How much of a page is read: the tags are in its head.
const PAGE_LIMIT: u64 = 512 * 1024;
const IMAGE_LIMIT: u64 = 8 * 1024 * 1024;
/// Larger pictures are not decoded at all.
const IMAGE_MAX_SIDE: u32 = 8192;
/// The small picture carried inside the message.
const THUMBNAIL_SIDE: u32 = 192;
/// The larger picture phones show across the message, for pictures at least
/// this wide and no taller than wide.
const IMAGE_SIDE: u32 = 1200;
const WIDE_MIN_WIDTH: u32 = 300;
const TITLE_LIMIT: usize = 200;
const DESCRIPTION_LIMIT: usize = 300;

/// Sites that keep their tags for known previewers answer WhatsApp.
const USER_AGENT: &str = concat!("WhatsApp/2 ZapFast/", env!("CARGO_PKG_VERSION"));

/// The latest request: an earlier one still waiting out [`PAUSE`] is dropped.
static LATEST: AtomicU64 = AtomicU64::new(0);

/// Fetches the preview of `link` once typing pauses, and reports it back as
/// [`Command::LinkPreviewFetched`].
pub(super) fn request(link: String, commands: mpsc::UnboundedSender<Command>) {
    let turn = LATEST.fetch_add(1, Ordering::SeqCst) + 1;
    tokio::spawn(async move {
        tokio::time::sleep(PAUSE).await;
        if LATEST.load(Ordering::SeqCst) != turn {
            return;
        }
        let card = tokio::task::spawn_blocking({
            let link = link.clone();
            move || fetch(&link)
        })
        .await
        .ok()
        .flatten();
        let _ = commands.send(Command::LinkPreviewFetched { link, card });
    });
}

/// Carries a preview in a text message, which then goes as extended text.
pub(super) fn attach(message: &mut wa::Message, card: &LinkCard) {
    if let Some(text) = message.conversation.take() {
        message.extended_text_message = MessageField::some(wa::message::ExtendedTextMessage {
            text: Some(text),
            ..Default::default()
        });
    }
    let Some(extended) = message.extended_text_message.as_option_mut() else {
        return;
    };
    extended.matched_text = Some(card.link.clone());
    extended.title = card.title.clone();
    extended.description = card.description.clone();
    extended.jpeg_thumbnail = card.thumbnail.clone();
    extended.preview_type = Some(wa::message::extended_text_message::PreviewType::NONE);
}

/// The preview as our own message shows it, like a received one.
pub(super) fn shown(card: &LinkCard) -> Option<LinkPreview> {
    Some(LinkPreview {
        url: crate::safety::preview_url(&card.link)?,
        title: card.title.clone(),
        description: card.description.clone(),
    })
}

/// Uploads the larger picture and names it in the message. Without it, phones
/// show the small picture beside the title, so a failed upload still sends.
pub(super) async fn upload_image(client: &Client, message: &mut wa::Message, image: LinkImage) {
    let upload = match client
        .upload(
            image.jpeg,
            MediaType::LinkThumbnail,
            UploadOptions::default(),
        )
        .await
    {
        Ok(upload) => upload,
        Err(error) => {
            log::warn!("could not upload a link preview picture: {error}");
            return;
        }
    };
    if let Some(extended) = message.extended_text_message.as_option_mut() {
        extended.thumbnail_direct_path = Some(upload.direct_path);
        extended.thumbnail_sha256 = Some(upload.file_sha256.to_vec());
        extended.thumbnail_enc_sha256 = Some(upload.file_enc_sha256.to_vec());
        extended.media_key = Some(upload.media_key.to_vec());
        extended.media_key_timestamp = Some(upload.media_key_timestamp);
        extended.thumbnail_width = Some(image.width);
        extended.thumbnail_height = Some(image.height);
    }
}

/// Reads the page behind `link` and its picture. `None` when the page cannot
/// be read, is not HTML, or says nothing about itself.
fn fetch(link: &str) -> Option<LinkCard> {
    let url = crate::safety::preview_url(link)?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .proxy(crate::proxy::ureq_proxy())
        .timeout_global(Some(TIMEOUT))
        .max_redirects(5)
        .user_agent(USER_AGENT)
        .build()
        .into();
    let mut response = agent
        .get(&url)
        .header("Accept", "text/html,application/xhtml+xml;q=0.9,*/*;q=0.5")
        .call()
        .ok()?;
    let page = reqwest::Url::parse(&response.get_uri().to_string()).ok()?;
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !content_type.is_empty() && !content_type.contains("html") {
        return None;
    }
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(PAGE_LIMIT)
        .read_to_end(&mut bytes)
        .ok()?;
    let tags = Tags::parse(&decode(&bytes, &content_type));
    let (thumbnail, image) = tags
        .image
        .as_deref()
        .and_then(|source| page.join(source).ok())
        .filter(|source| matches!(source.scheme(), "http" | "https"))
        .and_then(|source| pictures(&agent, source.as_str()))
        .map_or((None, None), |(thumbnail, image)| (Some(thumbnail), image));
    if tags.title.is_none() && tags.description.is_none() && thumbnail.is_none() {
        return None;
    }
    Some(LinkCard {
        link: link.to_owned(),
        title: tags.title,
        description: tags.description,
        thumbnail,
        image,
    })
}

/// The small JPEG, and the larger one for a wide enough picture.
fn pictures(agent: &ureq::Agent, url: &str) -> Option<(Vec<u8>, Option<LinkImage>)> {
    let bytes = agent
        .get(url)
        .header("Accept", "image/*")
        .call()
        .ok()?
        .body_mut()
        .with_config()
        .limit(IMAGE_LIMIT)
        .read_to_vec()
        .ok()?;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(IMAGE_MAX_SIDE);
    limits.max_image_height = Some(IMAGE_MAX_SIDE);
    reader.limits(limits);
    let picture = on_white(reader.decode().ok()?);
    let thumbnail = encode_jpeg(&picture.thumbnail(THUMBNAIL_SIDE, THUMBNAIL_SIDE), 70).ok()?;
    let image = (picture.width() >= WIDE_MIN_WIDTH && picture.width() >= picture.height())
        .then(|| {
            let wide = picture.thumbnail(IMAGE_SIDE, IMAGE_SIDE);
            encode_jpeg(&wide, 80).ok().map(|jpeg| LinkImage {
                jpeg,
                width: wide.width(),
                height: wide.height(),
            })
        })
        .flatten();
    Some((thumbnail, image))
}

/// JPEG has no transparency: a see-through logo goes on white, not black.
fn on_white(picture: image::DynamicImage) -> image::DynamicImage {
    if !picture.color().has_alpha() {
        return picture;
    }
    let rgba = picture.to_rgba8();
    let blend = |channel: u8, alpha: u8| {
        ((u32::from(channel) * u32::from(alpha) + 255 * (255 - u32::from(alpha))) / 255) as u8
    };
    image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(
        rgba.width(),
        rgba.height(),
        |x, y| {
            let [r, g, b, a] = rgba.get_pixel(x, y).0;
            image::Rgb([blend(r, a), blend(g, a), blend(b, a)])
        },
    ))
}

/// The page as text: UTF-8 unless the answer or the page says Latin-1.
fn decode(bytes: &[u8], content_type: &str) -> String {
    let declared = charset(content_type).or_else(|| {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(2048)]).to_ascii_lowercase();
        charset(&head)
    });
    match declared.as_deref() {
        Some("iso-8859-1" | "latin1" | "windows-1252") => {
            bytes.iter().map(|&byte| char::from(byte)).collect()
        }
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// The value after the first `charset=` in lowercase `text`.
fn charset(text: &str) -> Option<String> {
    let rest = &text[text.find("charset=")? + "charset=".len()..];
    let rest = rest.trim_start_matches(['"', '\'']);
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| rest[..end].to_owned())
}

/// What a page's head says about it.
#[derive(Debug, Default, PartialEq)]
struct Tags {
    title: Option<String>,
    description: Option<String>,
    /// The picture's address, maybe relative to the page.
    image: Option<String>,
}

impl Tags {
    fn parse(html: &str) -> Self {
        // ASCII lowercasing keeps every byte offset, so positions found in
        // `lower` cut `html` too.
        let lower = html.to_ascii_lowercase();
        let end = lower.find("</head").unwrap_or(lower.len());
        let (html, lower) = (&html[..end], &lower[..end]);
        let mut properties: Vec<(String, String)> = Vec::new();
        let mut at = 0;
        while let Some(start) = lower[at..].find("<meta").map(|found| at + found) {
            let close = lower[start..]
                .find('>')
                .map_or(lower.len(), |found| start + found);
            let attributes = attributes(&html[start + "<meta".len()..close]);
            let value = |name: &str| {
                attributes
                    .iter()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value.as_str())
            };
            if let Some(key) = value("property").or_else(|| value("name"))
                && let Some(content) = value("content")
            {
                properties.push((key.to_ascii_lowercase(), content.to_owned()));
            }
            at = close;
        }
        let property = |names: &[&str], limit: usize| {
            names.iter().find_map(|name| {
                properties
                    .iter()
                    .filter(|(key, _)| key == name)
                    .find_map(|(_, content)| clean(content, limit))
            })
        };
        let title = property(&["og:title", "twitter:title"], TITLE_LIMIT).or_else(|| {
            let open = lower.find("<title")?;
            let start = open + lower[open..].find('>')? + 1;
            let end = start + lower[start..].find("</title")?;
            clean(&html[start..end], TITLE_LIMIT)
        });
        Self {
            title,
            description: property(
                &["og:description", "twitter:description", "description"],
                DESCRIPTION_LIMIT,
            ),
            image: property(
                &[
                    "og:image:secure_url",
                    "og:image",
                    "og:image:url",
                    "twitter:image",
                    "twitter:image:src",
                ],
                usize::MAX,
            ),
        }
    }
}

/// A tag's attributes as lowercase names and raw values.
fn attributes(tag: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut rest = tag;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
        let Some(first) = rest.chars().next() else {
            break;
        };
        let name_end = rest
            .find(|c: char| c.is_whitespace() || c == '=' || c == '/')
            .unwrap_or(rest.len());
        if name_end == 0 {
            rest = &rest[first.len_utf8()..];
            continue;
        }
        let name = rest[..name_end].to_ascii_lowercase();
        rest = rest[name_end..].trim_start();
        let mut value = "";
        if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            if let Some(quote) = after.chars().next().filter(|c| *c == '"' || *c == '\'') {
                let inside = &after[1..];
                let end = inside.find(quote).unwrap_or(inside.len());
                value = &inside[..end];
                rest = inside.get(end + 1..).unwrap_or_default();
            } else {
                let end = after.find(char::is_whitespace).unwrap_or(after.len());
                value = &after[..end];
                rest = &after[end..];
            }
        }
        found.push((name, value.to_owned()));
    }
    found
}

/// Entities decoded, runs of spaces made one, and at most `limit` characters.
fn clean(text: &str, limit: usize) -> Option<String> {
    let text = unescape(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        return None;
    }
    if text.chars().count() <= limit {
        return Some(text);
    }
    let mut short: String = text.chars().take(limit.saturating_sub(1)).collect();
    short.truncate(short.trim_end().len());
    short.push('…');
    Some(short)
}

/// Decodes HTML character references; an unknown one stays as written.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let decoded = after
            .find(';')
            .filter(|end| *end <= 10)
            .and_then(|end| entity(&after[..end]).map(|c| (c, end)));
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    if let Some(number) = name.strip_prefix('#') {
        let code = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse().ok()?,
        };
        return char::from_u32(code);
    }
    let named = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "hellip" => '…',
        "mdash" => '—',
        "ndash" => '–',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "laquo" => '«',
        "raquo" => '»',
        "middot" => '·',
        "bull" => '•',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "szlig" => 'ß',
        _ => return accented(name),
    };
    Some(named)
}

/// Accented Latin letters, such as `atilde` and `Ccedil`.
fn accented(name: &str) -> Option<char> {
    const LETTERS: &[(&str, char)] = &[
        ("aacute", 'á'),
        ("agrave", 'à'),
        ("acirc", 'â'),
        ("atilde", 'ã'),
        ("auml", 'ä'),
        ("eacute", 'é'),
        ("egrave", 'è'),
        ("ecirc", 'ê'),
        ("euml", 'ë'),
        ("iacute", 'í'),
        ("igrave", 'ì'),
        ("icirc", 'î'),
        ("iuml", 'ï'),
        ("oacute", 'ó'),
        ("ograve", 'ò'),
        ("ocirc", 'ô'),
        ("otilde", 'õ'),
        ("ouml", 'ö'),
        ("uacute", 'ú'),
        ("ugrave", 'ù'),
        ("ucirc", 'û'),
        ("uuml", 'ü'),
        ("ccedil", 'ç'),
        ("ntilde", 'ñ'),
    ];
    let first = name.chars().next()?;
    let lower = format!(
        "{}{}",
        first.to_ascii_lowercase(),
        &name[first.len_utf8()..]
    );
    let letter = LETTERS
        .iter()
        .find(|(entity, _)| *entity == lower)
        .map(|(_, letter)| *letter)?;
    if first.is_ascii_uppercase() {
        letter.to_uppercase().next()
    } else {
        Some(letter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_graph_tags_come_before_the_title_and_description() {
        let tags = Tags::parse(
            "<html><head><title>Plain</title>\
             <meta name=\"description\" content=\"Plain description\">\
             <meta property=\"og:title\" content=\"Caf&eacute; &amp; P&atilde;o\" />\
             <meta content='/pic.png' property='og:image'>\
             </head><body><meta property=\"og:description\" content=\"Body\"></body></html>",
        );
        assert_eq!(
            tags,
            Tags {
                title: Some("Café & Pão".into()),
                description: Some("Plain description".into()),
                image: Some("/pic.png".into()),
            }
        );
    }

    #[test]
    fn a_page_without_tags_keeps_its_title() {
        let tags = Tags::parse("<TITLE>\n  Só   o título &#8212; &#x41;\n</TITLE>");
        assert_eq!(tags.title.as_deref(), Some("Só o título — A"));
        assert_eq!(tags.description, None);
        assert_eq!(tags.image, None);
    }

    #[test]
    fn unknown_references_stay_as_written() {
        assert_eq!(
            unescape("a &unknown; b & c &Ccedil;"),
            "a &unknown; b & c Ç"
        );
    }

    #[test]
    fn long_text_is_shortened_with_an_ellipsis() {
        assert_eq!(clean("one two three", 8).as_deref(), Some("one two…"));
        assert_eq!(clean("   ", 8), None);
    }

    #[test]
    fn the_charset_comes_from_the_answer_or_the_page() {
        assert_eq!(decode(b"caf\xe9", "text/html; charset=iso-8859-1"), "café");
        assert_eq!(
            decode(b"<meta charset=\"windows-1252\">caf\xe9", ""),
            "<meta charset=\"windows-1252\">café"
        );
        assert_eq!(decode("café".as_bytes(), "text/html"), "café");
    }
}
