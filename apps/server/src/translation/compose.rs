use super::*;

impl TranslationService {
    pub async fn compose_ai_message(
        &self,
        prompt: &str,
        reply_context: Option<(&str, &str)>,
        reply_image: Option<(&str, &str)>,
    ) -> Result<(String, UsageInfo)> {
        if prompt.trim().is_empty() {
            anyhow::bail!("Prompt cannot be empty");
        }
        if prompt.len() > 1000 {
            anyhow::bail!("Prompt is too long (max 1000 characters)");
        }

        let instructions = r#"You are a helpful assistant composing WhatsApp messages. Your task is to write a message based on the user's request.

IMPORTANT RULES:
1. Keep your response SHORT and appropriate for a chat message (max 500 characters)
2. Write ONLY the message content - no explanations, no quotes, no "Here's a message:" prefixes
3. Be conversational and natural, matching the tone requested
4. Do not include anything harmful, offensive, or inappropriate
5. If the request is unclear, write a friendly, neutral message
6. Do not pretend to be someone specific or impersonate anyone
7. Do not include private information or make up facts about real people
8. If an image is provided, you can reference what you see in it when composing your reply"#;

        let text_content = if let Some((sender, text)) = reply_context {
            format!(
                "The user is replying to this message from {}:\n\"{}\"\n\nUser request for their reply: {}",
                sender,
                text,
                prompt
            )
        } else {
            format!("User request: {}", prompt)
        };

        let input = if let Some((media_type, base64_data)) = reply_image {
            let mut content = vec![
                json!({
                    "type": "input_image",
                    "image_url": Self::build_data_url(media_type, base64_data),
                }),
                json!({
                    "type": "input_text",
                    "text": text_content,
                }),
            ];

            if let Some((sender, _)) = reply_context {
                content.insert(
                    1,
                    json!({
                        "type": "input_text",
                        "text": format!("The above image was sent by {}.", sender),
                    }),
                );
            }

            json!([{
                "role": "user",
                "content": content,
            }])
        } else {
            json!(text_content)
        };

        let (composed, usage_info) = self
            .request_text_output(
                &self.high_end_model,
                HIGH_END_PRICING,
                instructions,
                input,
                300,
                Some("medium"),
                Some("low"),
                false,
            )
            .await?;

        let composed = Self::truncate_for_display(composed.trim().to_string(), 500);

        info!(
            "AI compose usage: {} in ({} cached), {} out, ${:.6}",
            usage_info.input_tokens,
            usage_info.cached_input_tokens,
            usage_info.output_tokens,
            usage_info.cost_usd
        );

        Ok((composed, usage_info))
    }

    pub async fn compose_styled_reply(
        &self,
        message_to_reply: &crate::storage::StoredMessage,
        recent_conversation: &[crate::storage::StoredMessage],
        global_style: &crate::storage::StyleProfile,
        contact_style: Option<&crate::storage::StyleProfile>,
        my_examples: &[crate::storage::StoredMessage],
    ) -> Result<(String, UsageInfo)> {
        let reply_to_text = message_to_reply
            .original_text
            .clone()
            .or_else(|| message_to_reply.translated_text.clone())
            .or_else(|| {
                message_to_reply.content.as_ref().and_then(|c| {
                    c.get("body")
                        .and_then(|v| v.as_str().map(String::from))
                        .or_else(|| c.get("caption").and_then(|v| v.as_str().map(String::from)))
                })
            })
            .unwrap_or_else(|| "[No text content]".to_string());
        let reply_to_text = Self::truncate_for_display(reply_to_text, 500);

        let sender_name = message_to_reply
            .sender_name
            .clone()
            .or_else(|| message_to_reply.contact_name.clone())
            .unwrap_or_else(|| "Someone".to_string());

        let conversation_context = Self::format_conversation(recent_conversation);
        let my_examples_formatted = Self::format_my_examples(my_examples);

        let contact_style_section = if let Some(cs) = contact_style {
            format!(
                "## MY STYLE WITH THIS SPECIFIC CONTACT:\n{}\n",
                cs.profile_text
            )
        } else {
            "## MY STYLE WITH THIS SPECIFIC CONTACT:\nNo specific style data for this contact yet. Use my general style.\n".to_string()
        };

        let prompt = format!(
            r#"Write a WhatsApp reply AS ME. You must sound EXACTLY like my example messages below.

## MY ACTUAL MESSAGES (COPY THIS STYLE EXACTLY):
{}

## RECENT CHAT FOR CONTEXT:
{}

## REPLYING TO:
{}: "{}"

## STYLE NOTES:
{}
{}

## ABSOLUTE RULES - FOLLOW THESE OR FAIL:
1. BE SHORT. Real WhatsApp messages are 1-2 sentences max, not paragraphs
2. DO NOT start with "Oh" or "Ah" or any filler words - that's AI speak
3. DO NOT over-explain feelings ("I love that", "That's really interesting") - just react naturally
4. DO NOT write in complete formal sentences if my examples don't
5. DO NOT be more enthusiastic or wordy than my examples show
6. COPY my emoji patterns exactly - if I use "😂" use that, if I don't use emojis, DON'T add them
7. COPY my punctuation - if I skip full stops, skip them. If I use "haha" vs "lol", match it
8. COPY my greeting/sign-off style (xxxxxx, etc) if I use them
9. Sound like a REAL HUMAN texting a friend, not an AI assistant being helpful
10. Output ONLY the message text, nothing else

Write my reply (keep it short and casual like my examples):"#,
            my_examples_formatted,
            conversation_context,
            sender_name,
            reply_to_text,
            global_style.profile_text,
            contact_style_section
        );

        debug!(
            "AI reply prompt length: {} chars, examples: {}, conversation: {} msgs",
            prompt.len(),
            my_examples.len(),
            recent_conversation.len()
        );

        let (mut reply, mut usage_info) = self
            .request_text_output(
                &self.high_end_model,
                HIGH_END_PRICING,
                "Generate a short WhatsApp reply that follows the user's style exactly. Output only the reply text.",
                json!(prompt.clone()),
                150,
                Some("none"),
                Some("low"),
                false,
            )
            .await?;

        if reply.trim().is_empty() {
            warn!("Styled reply returned no text, retrying with a simpler prompt");
            let retry_prompt = format!(
                "{}\n\nFINAL REQUIREMENT: Reply with one short WhatsApp message right now. Do not leave the answer blank.",
                prompt
            );
            let (retry_reply, retry_usage) = self
                .request_text_output(
                    &self.high_end_model,
                    HIGH_END_PRICING,
                    "Write exactly one short WhatsApp reply in the user's style. Output only the reply text.",
                    json!(retry_prompt),
                    220,
                    Some("none"),
                    Some("low"),
                    false,
                )
                .await?;
            reply = retry_reply;
            usage_info = Self::combine_usage(&usage_info, &retry_usage);
        }

        let reply = Self::truncate_for_display(reply.trim().to_string(), 500);

        if reply.is_empty() {
            anyhow::bail!("OpenAI returned an empty styled reply");
        }

        info!(
            "Styled reply generated: {} chars, {} in ({} cached), {} out, ${:.6}",
            reply.len(),
            usage_info.input_tokens,
            usage_info.cached_input_tokens,
            usage_info.output_tokens,
            usage_info.cost_usd
        );

        Ok((reply, usage_info))
    }

    pub(super) fn format_conversation(messages: &[crate::storage::StoredMessage]) -> String {
        if messages.is_empty() {
            return "No recent messages.".to_string();
        }

        messages
            .iter()
            .map(|m| {
                let sender = if m.is_from_me {
                    "Me".to_string()
                } else {
                    m.sender_name
                        .clone()
                        .or_else(|| m.contact_name.clone())
                        .unwrap_or_else(|| "Them".to_string())
                };

                let text = m
                    .original_text
                    .clone()
                    .or_else(|| m.translated_text.clone())
                    .or_else(|| {
                        m.content.as_ref().and_then(|c| {
                            c.get("body")
                                .and_then(|v| v.as_str().map(String::from))
                                .or_else(|| {
                                    c.get("caption").and_then(|v| v.as_str().map(String::from))
                                })
                        })
                    })
                    .unwrap_or_else(|| format!("[{}]", m.content_type));

                format!("{}: {}", sender, Self::truncate_for_display(text, 200))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(super) fn format_my_examples(messages: &[crate::storage::StoredMessage]) -> String {
        if messages.is_empty() {
            return "No previous messages to this contact yet.".to_string();
        }

        messages
            .iter()
            .filter_map(|m| {
                m.original_text.clone().or_else(|| {
                    m.content.as_ref().and_then(|c| {
                        c.get("body")
                            .and_then(|v| v.as_str().map(String::from))
                            .or_else(|| c.get("caption").and_then(|v| v.as_str().map(String::from)))
                    })
                })
            })
            .enumerate()
            .map(|(i, text)| format!("{}. \"{}\"", i + 1, Self::truncate_for_display(text, 200)))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
