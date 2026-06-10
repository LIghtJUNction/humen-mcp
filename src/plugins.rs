#[derive(Clone, Debug, Default, Serialize)]
struct PluginRegistry {
    plugins: Vec<LoadedPlugin>,
}

#[derive(Clone, Debug, Serialize)]
struct LoadedPlugin {
    manifest: HumenPluginManifest,
    source: String,
}

#[derive(Clone, Debug, Deserialize)]
struct TemplateRequestArgs {
    template: String,
    #[serde(default)]
    variables: HashMap<String, Value>,
    title: Option<String>,
    prompt: Option<String>,
    #[serde(default)]
    choices: Vec<String>,
    #[serde(default)]
    steps: Vec<String>,
    timeout_seconds: Option<u64>,
    #[serde(default)]
    background: bool,
}

impl PluginRegistry {
    fn template(&self, id: &str) -> Option<(&LoadedPlugin, &RequestTemplate)> {
        let (plugin_id, template_id) = split_plugin_item_id(id)?;
        self.plugins.iter().find_map(|plugin| {
            if plugin.manifest.id != plugin_id {
                return None;
            }
            plugin
                .manifest
                .request_templates
                .iter()
                .find(|template| template.id == template_id)
                .map(|template| (plugin, template))
        })
    }

    fn plugin_summary(&self) -> Value {
        let plugins: Vec<_> = self
            .plugins
            .iter()
            .map(|plugin| {
                json!({
                    "id": plugin.manifest.id,
                    "name": plugin.manifest.name,
                    "version": plugin.manifest.version,
                    "description": plugin.manifest.description,
                    "author": plugin.manifest.author,
                    "source": plugin.source,
                    "request_templates": plugin.manifest.request_templates,
                    "route_strategies": plugin.manifest.route_strategies,
                    "scoring_rules": plugin.manifest.scoring_rules,
                    "channels": plugin.manifest.channels,
                })
            })
            .collect();
        json!({
            "plugins": plugins,
            "counts": {
                "plugins": self.plugins.len(),
                "request_templates": self.plugins.iter().map(|plugin| plugin.manifest.request_templates.len()).sum::<usize>(),
                "route_strategies": self.plugins.iter().map(|plugin| plugin.manifest.route_strategies.len()).sum::<usize>(),
                "scoring_rules": self.plugins.iter().map(|plugin| plugin.manifest.scoring_rules.len()).sum::<usize>(),
                "channels": self.plugins.iter().map(|plugin| plugin.manifest.channels.len()).sum::<usize>()
            }
        })
    }
}

fn load_plugins(plugin_dir: &str) -> PluginRegistry {
    if plugin_dir.is_empty() {
        return PluginRegistry::default();
    }

    let Ok(entries) = fs::read_dir(plugin_dir) else {
        warn!(plugin_dir, "plugin directory is not readable");
        return PluginRegistry::default();
    };

    let mut loaded = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || !is_plugin_manifest_file(&path) {
            continue;
        }
        match read_plugin_manifest(&path) {
            Ok(manifest) => {
                info!(
                    plugin_id = manifest.id,
                    source = %path.display(),
                    "loaded humen plugin"
                );
                loaded.push(LoadedPlugin {
                    manifest,
                    source: path.display().to_string(),
                });
            }
            Err(err) => {
                warn!(source = %path.display(), error = %err, "failed to load humen plugin");
            }
        }
    }
    loaded.sort_by(|left, right| left.manifest.id.cmp(&right.manifest.id));
    PluginRegistry { plugins: loaded }
}

fn is_plugin_manifest_file(path: &std::path::Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("json" | "toml")
    )
}

fn read_plugin_manifest(path: &std::path::Path) -> anyhow::Result<HumenPluginManifest> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let manifest: HumenPluginManifest = match path.extension().and_then(|extension| extension.to_str()) {
        Some("toml") => toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))?,
        _ => serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?,
    };
    manifest
        .validate()
        .map_err(|err| anyhow::anyhow!("invalid plugin manifest: {err}"))?;
    Ok(manifest)
}

fn split_plugin_item_id(id: &str) -> Option<(&str, &str)> {
    id.split_once('/')
}

fn create_request_from_template_args(
    registry: &PluginRegistry,
    args: TemplateRequestArgs,
) -> Result<CreateHumanRequest, ApiError> {
    let (_, template) = registry
        .template(&args.template)
        .ok_or_else(|| ApiError::bad_request("plugin template not found"))?;
    let title = args.title.unwrap_or_else(|| template.title.clone());
    let prompt_template = if let Some(prompt) = args.prompt {
        prompt
    } else if template.prompt_template.trim().is_empty() {
        template.description.clone()
    } else {
        template.prompt_template.clone()
    };
    let choices = if args.choices.is_empty() {
        template.choices.clone()
    } else {
        args.choices
    };
    let steps = if args.steps.is_empty() {
        template.steps.clone()
    } else {
        args.steps
    };

    Ok(CreateHumanRequest {
        kind: task_kind_from_sdk(&template.kind),
        title: render_template_text(&title, &args.variables),
        prompt: render_template_text(&prompt_template, &args.variables),
        choices: choices
            .into_iter()
            .map(|choice| render_template_text(&choice, &args.variables))
            .collect(),
        image_url: None,
        image_base64: None,
        image_mime_type: None,
        steps: steps
            .into_iter()
            .map(|step| render_template_text(&step, &args.variables))
            .collect(),
        timeout_seconds: args.timeout_seconds.or(template.timeout_seconds).unwrap_or_else(default_timeout),
        background: args.background,
        target_human_email: None,
    })
}

fn render_template_text(template: &str, variables: &HashMap<String, Value>) -> String {
    let mut rendered = template.to_string();
    for (key, value) in variables {
        if is_template_variable_name(key) {
            let token = format!("{{{{{key}}}}}");
            rendered = rendered.replace(&token, &value_to_template_text(value));
        }
    }
    rendered
}

fn is_template_variable_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn value_to_template_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

fn task_kind_from_sdk(kind: &HumenTaskKind) -> TaskKind {
    match kind {
        HumenTaskKind::Choice => TaskKind::Choice,
        HumenTaskKind::Judgment => TaskKind::Judgment,
        HumenTaskKind::Text => TaskKind::Text,
        HumenTaskKind::ImageReview => TaskKind::ImageReview,
        HumenTaskKind::Steps => TaskKind::Steps,
    }
}

fn list_humen_plugins_schema() -> Value {
    json!({
        "type": "object",
        "properties": {}
    })
}

fn create_humen_request_from_template_schema() -> Value {
    json!({
        "type": "object",
        "required": ["template"],
        "properties": {
            "template": {
                "type": "string",
                "description": "Template id in the form plugin-id/template-id."
            },
            "variables": {
                "type": "object",
                "description": "Values substituted into {{name}} placeholders."
            },
            "title": {
                "type": "string",
                "description": "Optional title override."
            },
            "prompt": {
                "type": "string",
                "description": "Optional prompt override."
            },
            "choices": {
                "type": "array",
                "items": { "type": "string" }
            },
            "steps": {
                "type": "array",
                "items": { "type": "string" }
            },
            "timeout_seconds": {
                "type": "integer",
                "minimum": 30,
                "maximum": 86400
            },
            "background": {
                "type": "boolean",
                "default": true
            }
        }
    })
}
