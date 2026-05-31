use futures::future::join_all;
use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use crate::agent::tools::{ToolError, check_perm};
use crate::extras::subagents::builder;
use crate::extras::subagents::with_config;
use crate::permission::ask::AskSender;
use crate::permission::checker::PermCheck;

#[derive(Deserialize)]
pub struct TaskArgs {
    /// One or more exploration prompts. When multiple are provided,
    /// they are explored in parallel subagents and results are combined.
    pub prompts: Vec<String>,
}

pub struct TaskTool {
    permission: Option<PermCheck>,
    ask_tx: Option<AskSender>,
}

impl TaskTool {
    pub fn new(permission: Option<PermCheck>, ask_tx: Option<AskSender>) -> Self {
        Self { permission, ask_tx }
    }
}

impl Tool for TaskTool {
    const NAME: &'static str = "task";
    type Error = ToolError;
    type Args = TaskArgs;
    type Output = String;

    async fn definition(&self, _p: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Delegate read-only codebase exploration to a subagent. \
Provide one or more prompts describing what to investigate. \
Multiple prompts are explored in parallel. \
The subagent can read, grep, glob, list directories, and access memory. \
Returns a summary of findings for each prompt."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompts": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "One or more exploration tasks (parallel when multiple)"
                    }
                },
                "required": ["prompts"]
            }),
        }
    }

    async fn call(&self, args: TaskArgs) -> Result<String, ToolError> {
        if args.prompts.is_empty() {
            return Err(ToolError::Msg("task: prompts must not be empty".into()));
        }

        check_perm(
            &self.permission,
            &self.ask_tx,
            Self::NAME,
            &args.prompts.join(" | "),
        )
        .await?;

        let (client, model_name, max_turns) = with_config(|cfg| {
            (cfg.client.clone(), cfg.model_name.clone(), cfg.max_turns)
        });

        let mut handles = Vec::with_capacity(args.prompts.len());
        for (i, prompt_text) in args.prompts.iter().enumerate() {
            let prompt_text = prompt_text.clone();
            let model = client.completion_model(model_name.clone());
            handles.push(tokio::spawn(async move {
                let agent = builder::build_explore_agent(model, max_turns).await;
                let result = agent.run_subagent(&prompt_text, max_turns).await;
                (i, prompt_text, result)
            }));
        }

        let results = join_all(handles).await;

        let mut outputs: Vec<(usize, String, String)> = Vec::new();
        for r in results {
            match r {
                Ok((i, prompt_text, Ok(response))) => {
                    outputs.push((i, prompt_text, response));
                }
                Ok((i, prompt_text, Err(e))) => {
                    outputs.push((i, prompt_text, format!("[error: {}]", e)));
                }
                Err(e) => {
                    outputs.push((
                        outputs.len(),
                        "(unknown)".to_string(),
                        format!("[task panicked: {}]", e),
                    ));
                }
            }
        }

        outputs.sort_by_key(|(i, _, _)| *i);

        let mut combined = String::new();
        for (idx, (_, prompt_text, response)) in outputs.iter().enumerate() {
            if outputs.len() > 1 {
                if idx > 0 {
                    combined.push('\n');
                }
                let label = prompt_text.chars().take(60).collect::<String>();
                combined.push_str(&format!("## Task {}: {}\n\n", idx + 1, label));
            }
            combined.push_str(response);
            if !combined.ends_with('\n') {
                combined.push('\n');
            }
        }

        Ok(combined)
    }
}
