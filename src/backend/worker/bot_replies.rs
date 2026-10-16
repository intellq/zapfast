//! Meta AI messages.
//!
//! A message that invokes Meta AI arrives wrapped in `bot_invoke_message`, and
//! Meta AI answers with a `rich_response_message`: a list of parts, each text,
//! code, a table, a formula, an image, or a map. The parts that are text at
//! heart become one text message, code in a WhatsApp monospace block; a reply
//! with none of them shows as an unsupported message rather than vanishing.

use super::*;
use whatsapp_rust::waproto::whatsapp::AIRichResponseMessage;
use whatsapp_rust::waproto::whatsapp::AIRichResponseSubMessage;

/// What an unreadable reply is called in its placeholder.
const UNREADABLE: &str = "Meta AI reply";

/// The message inside a Meta AI invocation, unwrapped like the library's own
/// wrappers.
pub(super) fn invoked(base: &wa::Message) -> Option<&wa::Message> {
    base.bot_invoke_message
        .as_option()
        .and_then(|wrapper| wrapper.message.as_option())
        .map(|inner| inner.get_base_message())
}

/// A rich response as the text it reads as.
pub(super) fn content(reply: &AIRichResponseMessage) -> Content {
    let parts: Vec<String> = reply.submessages.iter().filter_map(part).collect();
    if parts.is_empty() {
        return Content::Unsupported {
            what: UNREADABLE.to_owned(),
        };
    }
    Content::Text {
        text: parts.join("\n\n"),
        preview: None,
    }
}

fn part(part: &AIRichResponseSubMessage) -> Option<String> {
    let mut pieces = Vec::new();
    if let Some(text) = part
        .message_text
        .as_deref()
        .filter(|text| !text.trim().is_empty())
    {
        pieces.push(text.trim_end().to_owned());
    }
    if let Some(code) = part.code_metadata.as_option() {
        let body: String = code
            .code_blocks
            .iter()
            .filter_map(|block| block.code_content.as_deref())
            .collect();
        let body = body.trim_matches('\n');
        if !body.trim().is_empty() {
            pieces.push(format!("```\n{body}\n```"));
        }
    }
    if let Some(table) = part.table_metadata.as_option() {
        let rows = table
            .title
            .iter()
            .filter(|title| !title.trim().is_empty())
            .cloned()
            .chain(
                table
                    .rows
                    .iter()
                    .filter(|row| !row.items.is_empty())
                    .map(|row| row.items.join(" | ")),
            )
            .collect::<Vec<_>>();
        if !rows.is_empty() {
            pieces.push(rows.join("\n"));
        }
    }
    if let Some(text) = part
        .latex_metadata
        .as_option()
        .and_then(|latex| latex.text.as_deref())
        .filter(|text| !text.trim().is_empty())
    {
        pieces.push(text.trim_end().to_owned());
    }
    if let Some(text) = part
        .image_metadata
        .as_option()
        .and_then(|image| image.image_text.as_deref())
        .filter(|text| !text.trim().is_empty())
    {
        pieces.push(text.trim_end().to_owned());
    }
    (!pieces.is_empty()).then(|| pieces.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use whatsapp_rust::waproto::whatsapp::{
        AIRichResponseCodeMetadata, AIRichResponseMapMetadata, AIRichResponseTableMetadata,
        ai_rich_response_code_metadata::AIRichResponseCodeBlock,
        ai_rich_response_table_metadata::AIRichResponseTableRow,
    };

    fn text(text: &str) -> AIRichResponseSubMessage {
        {
            let mut proto = AIRichResponseSubMessage::default();
            proto.message_text = Some(text.into());
            proto
        }
    }

    fn reply(parts: Vec<AIRichResponseSubMessage>) -> wa::Message {
        {
            let mut proto2 = wa::Message::default();
            proto2.rich_response_message = MessageField::some({
                let mut proto = AIRichResponseMessage::default();
                proto.submessages = parts;
                proto
            });
            // Bot replies carry their bot metadata here.
            proto2.message_context_info = MessageField::some(wa::MessageContextInfo::default());
            proto2
        }
    }

    #[test]
    fn a_rich_reply_reads_as_one_text_message() {
        let code = {
            let mut proto3 = AIRichResponseSubMessage::default();
            proto3.code_metadata = MessageField::some({
                let mut proto2 = AIRichResponseCodeMetadata::default();
                proto2.code_language = Some("rust".into());
                proto2.code_blocks = ["fn main() ", "{}", "\n"]
                    .into_iter()
                    .map(|piece| {
                        let mut proto = AIRichResponseCodeBlock::default();
                        proto.code_content = Some(piece.into());
                        proto
                    })
                    .collect();
                proto2
            });
            proto3
        };
        let table = {
            let mut proto3 = AIRichResponseSubMessage::default();
            proto3.table_metadata = MessageField::some({
                let mut proto2 = AIRichResponseTableMetadata::default();
                proto2.title = Some("Planets".into());
                proto2.rows = vec![
                    {
                        let mut proto = AIRichResponseTableRow::default();
                        proto.items = vec!["Name".into(), "Moons".into()];
                        proto.is_heading = Some(true);
                        proto
                    },
                    {
                        let mut proto = AIRichResponseTableRow::default();
                        proto.items = vec!["Mars".into(), "2".into()];
                        proto.is_heading = None;
                        proto
                    },
                ];
                proto2
            });
            proto3
        };
        let map = {
            let mut proto = AIRichResponseSubMessage::default();
            proto.map_metadata = MessageField::some(AIRichResponseMapMetadata::default());
            proto
        };
        let message = reply(vec![text("Here you go:\n"), code, table, map]);
        assert_eq!(
            classify(&message),
            Some(Content::text(
                "Here you go:\n\n```\nfn main() {}\n```\n\nPlanets\nName | Moons\nMars | 2"
            ))
        );
    }

    #[test]
    fn a_reply_without_readable_parts_is_a_placeholder() {
        let map = {
            let mut proto = AIRichResponseSubMessage::default();
            proto.map_metadata = MessageField::some(AIRichResponseMapMetadata::default());
            proto
        };
        for message in [reply(Vec::new()), reply(vec![map, text("  ")])] {
            assert_eq!(
                classify(&message),
                Some(Content::Unsupported {
                    what: UNREADABLE.into()
                })
            );
        }
    }

    #[test]
    fn a_reply_keeps_its_quote() {
        let message = {
            let mut proto3 = wa::Message::default();
            proto3.rich_response_message = MessageField::some({
                let mut proto2 = AIRichResponseMessage::default();
                proto2.submessages = vec![text("Answer")];
                proto2.context_info = MessageField::some({
                    let mut proto = wa::ContextInfo::default();
                    proto.stanza_id = Some("question".into());
                    proto
                });
                proto2
            });
            proto3
        };
        assert_eq!(
            context_of(&message).and_then(|context| context.stanza_id.as_deref()),
            Some("question")
        );
    }

    #[test]
    fn an_invocation_shows_the_message_inside_it() {
        let invocation = {
            let mut proto5 = wa::Message::default();
            proto5.bot_invoke_message = MessageField::some({
                let mut proto4 = wa::message::FutureProofMessage::default();
                proto4.message = MessageField::some({
                    let mut proto3 = wa::Message::default();
                    proto3.extended_text_message = MessageField::some({
                        let mut proto2 = wa::message::ExtendedTextMessage::default();
                        proto2.text = Some("@Meta AI what is a moth?".into());
                        proto2.context_info = MessageField::some({
                            let mut proto = wa::ContextInfo::default();
                            proto.mentioned_jid = vec!["fixture@bot".into()];
                            proto
                        });
                        proto2
                    });
                    proto3
                });
                proto4
            });
            proto5.message_context_info = MessageField::some(wa::MessageContextInfo::default());
            proto5
        };
        assert_eq!(
            classify(&invocation),
            Some(Content::text("@Meta AI what is a moth?"))
        );
        assert_eq!(mentioned_of(&invocation), ["fixture@bot"]);
    }

    #[test]
    fn unknown_content_with_context_info_is_a_placeholder_and_metadata_alone_is_not() {
        let unknown = {
            let mut proto2 = wa::Message::default();
            proto2.music_message = MessageField::some({
                let mut proto = wa::message::MusicMessage::default();
                proto.song_uri = Some("https://example.invalid/song".into());
                proto
            });
            proto2.message_context_info = MessageField::some(wa::MessageContextInfo::default());
            proto2
        };
        assert!(matches!(
            classify(&unknown),
            Some(Content::Unsupported { .. })
        ));
        let metadata = {
            let mut proto = wa::Message::default();
            proto.message_context_info = MessageField::some(wa::MessageContextInfo::default());
            proto
        };
        assert_eq!(classify(&metadata), None);
        let key_share = {
            let mut proto = wa::Message::default();
            proto.sender_key_distribution_message = MessageField::some(Default::default());
            proto.message_context_info = MessageField::some(wa::MessageContextInfo::default());
            proto
        };
        assert_eq!(classify(&key_share), None);
    }
}
