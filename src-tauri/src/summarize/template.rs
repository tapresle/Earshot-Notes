//! Renders a model's own Jinja chat template, as embedded in its GGUF file.

use minijinja::{Environment, Error, ErrorKind, Value, context};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Message {
    pub role: &'static str,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system",
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user",
            content: content.into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("couldn't format the prompt with the model's chat template: {0}")]
pub struct TemplateError(String);

/// Special tokens some templates reference by name.
#[derive(Debug, Clone, Default)]
pub struct SpecialTokens {
    pub bos: String,
    pub eos: String,
}

/// Renders the conversation with the generation prompt appended, ready for the
/// model to write the assistant's reply. Templates that reject a system message
/// get its instructions merged into the first user message instead.
pub fn render(
    template: &str,
    messages: &[Message],
    tokens: &SpecialTokens,
) -> Result<String, TemplateError> {
    match render_once(template, messages, tokens) {
        Ok(prompt) => Ok(prompt),
        Err(first) if messages.first().is_some_and(|m| m.role == "system") => {
            render_once(template, &merge_system(messages), tokens).map_err(|_| first)
        }
        Err(error) => Err(error),
    }
}

fn render_once(
    template: &str,
    messages: &[Message],
    tokens: &SpecialTokens,
) -> Result<String, TemplateError> {
    let mut env = Environment::new();
    minijinja_contrib::add_to_environment(&mut env);
    env.set_unknown_method_callback(minijinja_contrib::pycompat::unknown_method_callback);
    env.add_function(
        "raise_exception",
        |message: String| -> Result<Value, Error> {
            Err(Error::new(ErrorKind::InvalidOperation, message))
        },
    );
    env.add_template("chat", template)
        .map_err(|e| TemplateError(e.to_string()))?;
    env.get_template("chat")
        .and_then(|t| {
            t.render(context! {
                messages => messages,
                add_generation_prompt => true,
                enable_thinking => false,
                bos_token => tokens.bos,
                eos_token => tokens.eos,
            })
        })
        .map_err(|e| TemplateError(e.to_string()))
}

fn merge_system(messages: &[Message]) -> Vec<Message> {
    let (system, rest) = messages.split_first().expect("checked non-empty");
    let mut merged: Vec<Message> = rest.to_vec();
    match merged.iter_mut().find(|m| m.role == "user") {
        Some(user) => user.content = format!("{}\n\n{}", system.content, user.content),
        None => merged.insert(0, Message::user(system.content.clone())),
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHATML: &str = "{% for message in messages %}{{'<|im_start|>' + message['role'] + '\n' + message['content'] + '<|im_end|>' + '\n'}}{% endfor %}{% if add_generation_prompt %}{{ '<|im_start|>assistant\n' }}{% endif %}";

    fn gemma4() -> String {
        std::fs::read_to_string("tests/fixtures/gemma4-chat-template.jinja").unwrap()
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("tests/fixtures/{name}-chat-template.jinja")).unwrap()
    }

    fn conversation() -> Vec<Message> {
        vec![
            Message::system("Summarize meetings."),
            Message::user("Notes and transcript"),
        ]
    }

    #[test]
    fn renders_chatml() {
        let prompt = render(CHATML, &conversation(), &SpecialTokens::default()).unwrap();

        assert_eq!(
            prompt,
            "<|im_start|>system\nSummarize meetings.<|im_end|>\n<|im_start|>user\nNotes and transcript<|im_end|>\n<|im_start|>assistant\n"
        );
    }

    #[test]
    fn renders_the_gemma_4_template() {
        let tokens = SpecialTokens {
            bos: "<bos>".into(),
            eos: "<turn|>".into(),
        };

        let prompt = render(&gemma4(), &conversation(), &tokens).unwrap();

        assert!(
            prompt.starts_with("<bos><|turn>system\nSummarize meetings."),
            "{prompt}"
        );
        assert!(
            prompt.contains("<|turn>user\nNotes and transcript<turn|>\n"),
            "{prompt}"
        );
        assert!(prompt.ends_with("<|turn>model\n"), "{prompt}");
        assert!(
            !prompt.contains("thought"),
            "thinking must stay off: {prompt}"
        );
    }

    #[test]
    fn renders_the_qwen3_instruct_2507_template() {
        let prompt = render(
            &fixture("qwen3-2507"),
            &conversation(),
            &SpecialTokens::default(),
        )
        .unwrap();

        assert_eq!(
            prompt,
            "<|im_start|>system\nSummarize meetings.<|im_end|>\n<|im_start|>user\nNotes and transcript<|im_end|>\n<|im_start|>assistant\n"
        );
    }

    #[test]
    fn renders_the_qwen3_template_with_thinking_off() {
        let prompt = render(
            &fixture("qwen3"),
            &conversation(),
            &SpecialTokens::default(),
        )
        .unwrap();

        assert!(
            prompt.starts_with("<|im_start|>system\nSummarize meetings.<|im_end|>\n"),
            "{prompt}"
        );
        assert!(
            prompt.ends_with("<|im_start|>assistant\n<think>\n\n</think>\n\n"),
            "thinking must stay off: {prompt}"
        );
    }

    #[test]
    fn renders_the_ministral_3_template() {
        let tokens = SpecialTokens {
            bos: "<s>".into(),
            eos: "</s>".into(),
        };

        let prompt = render(&fixture("ministral3"), &conversation(), &tokens).unwrap();

        assert_eq!(
            prompt,
            "<s>[SYSTEM_PROMPT]Summarize meetings.[/SYSTEM_PROMPT][INST]Notes and transcript[/INST]"
        );
    }

    #[test]
    fn renders_the_phi_4_mini_template() {
        let prompt = render(
            &fixture("phi4-mini"),
            &conversation(),
            &SpecialTokens::default(),
        )
        .unwrap();

        assert_eq!(
            prompt,
            "<|system|>Summarize meetings.<|end|><|user|>Notes and transcript<|end|><|assistant|>"
        );
    }

    #[test]
    fn merges_the_system_message_when_the_template_rejects_it() {
        let strict = "{% for message in messages %}{% if message['role'] == 'system' %}{{ raise_exception('System role not supported') }}{% endif %}[{{ message['role'] }}]{{ message['content'] }}{% endfor %}[assistant]";

        let prompt = render(strict, &conversation(), &SpecialTokens::default()).unwrap();

        assert_eq!(
            prompt,
            "[user]Summarize meetings.\n\nNotes and transcript[assistant]"
        );
    }

    #[test]
    fn reports_a_broken_template() {
        let error = render("{% for %}", &conversation(), &SpecialTokens::default()).unwrap_err();

        assert!(error.to_string().contains("chat template"));
    }
}
