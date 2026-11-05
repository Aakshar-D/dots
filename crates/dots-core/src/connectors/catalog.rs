//! The apps Dots can connect (connectors spec section 3): one entry per app, with the MCP server
//! it uses, who makes that server, how the person gets a key or signs in, and what a dot can do
//! with it. Every server here was checked on 2026-11-05; the spec records how.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::library::McpServerConfig;

str_enum!(AuthKind {
    Key => "key",
    OAuth => "oauth",
    ComingSoon => "coming_soon",
});

str_enum!(Trust {
    Official => "official",
    Community => "community",
});

str_enum!(Category {
    Communication => "communication",
    Work => "work",
    Developer => "developer",
    Search => "search",
    Files => "files",
    Business => "business",
});

/// How Dots reaches the app's MCP server. Values are `${VAR}` references to the connector's
/// Personal variables, never the values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum ServerTemplate {
    /// A hosted server. `bearer` names the variable sent as `Authorization: Bearer ${VAR}`.
    Http { url: String, bearer: Option<String> },
    /// A program Dots starts, with `env` (a value may hold `${VAR}`).
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    },
}

impl ServerTemplate {
    /// The Library config the connector's server item has.
    pub fn config(&self) -> McpServerConfig {
        match self {
            Self::Http { url, bearer } => McpServerConfig::Http {
                url: url.clone(),
                headers: bearer
                    .iter()
                    .map(|var| ("Authorization".to_string(), format!("Bearer ${{{var}}}")))
                    .collect(),
            },
            Self::Stdio { command, args, env } => McpServerConfig::Stdio {
                command: command.clone(),
                args: args.clone(),
                cwd: None,
                env: env.clone(),
            },
        }
    }
}

/// A program a stdio connector needs on this PC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeNeed {
    /// "Node.js".
    pub name: String,
    /// The program asked for its version (`node --version`).
    pub command: String,
    /// The lowest major version the server runs on.
    pub min_major: u32,
    /// Where to get it.
    pub download_url: String,
}

/// What a typed value must look like before it is stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FieldFormat {
    /// Anything that is not blank.
    Any,
    /// It starts with one of these.
    Prefix { prefixes: Vec<String> },
    /// An email address.
    Email,
    /// Exactly `length` letters once spaces are taken out (a Google app password).
    Letters { length: usize },
}

/// One value the person pastes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyField {
    /// The key the UI sends it under.
    pub key: String,
    pub label: String,
    /// The Personal variable it is stored as.
    pub variable: String,
    /// A secret goes to Credential Manager and is never shown again; a plain value (an email
    /// address) is kept in the database.
    pub secret: bool,
    pub placeholder: String,
    /// Shown under the field: "Starts with github_pat_ or ghp_".
    pub hint: String,
    pub format: FieldFormat,
    /// Spaces are taken out before it is stored (Google shows app passwords in groups of 4).
    #[serde(default)]
    pub strip_spaces: bool,
}

/// How to get the key, or what signing in does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instructions {
    pub steps: Vec<String>,
    /// The page the button opens.
    pub open_url: String,
    /// The button's words: "Open GitHub".
    pub open_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connector {
    /// Also the name of its Library MCP server.
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: Category,
    /// A key the UI maps to an icon.
    pub icon: String,
    /// Who makes the MCP server: the app's company, or the person or group who wrote it.
    pub maker: String,
    pub trust: Trust,
    /// Where the server's code or documentation is.
    pub source_url: String,
    pub auth: AuthKind,
    /// `None` only for a connector that is not available yet.
    pub server: Option<ServerTemplate>,
    pub runtime: Option<RuntimeNeed>,
    pub fields: Vec<KeyField>,
    pub instructions: Instructions,
    /// What a dot can do with it, one line each.
    pub abilities: Vec<String>,
    /// A note the panel shows: why it is not available, or a limit worth knowing.
    pub note: Option<String>,
}

impl Connector {
    /// The variable its sign-in keeps the access token in: `CONNECTOR_<ID>_ACCESS_TOKEN`.
    pub fn token_variable(&self) -> String {
        variable_for(&self.id, "ACCESS_TOKEN")
    }

    /// Every Personal variable the connector stores.
    pub fn variables(&self) -> Vec<String> {
        match self.auth {
            AuthKind::OAuth => vec![self.token_variable()],
            _ => self.fields.iter().map(|f| f.variable.clone()).collect(),
        }
    }
}

/// `CONNECTOR_<ID>_<FIELD>`, the id upper-cased with `-` as `_`.
pub fn variable_for(id: &str, field: &str) -> String {
    format!(
        "CONNECTOR_{}_{field}",
        id.to_ascii_uppercase().replace('-', "_")
    )
}

fn s(text: &str) -> String {
    text.to_string()
}

fn lines(items: &[&str]) -> Vec<String> {
    items.iter().map(|t| t.to_string()).collect()
}

fn node(min_major: u32) -> Option<RuntimeNeed> {
    Some(RuntimeNeed {
        name: s("Node.js"),
        command: s("node"),
        min_major,
        download_url: s(NODE_DOWNLOAD),
    })
}

/// Where Node.js is downloaded.
pub const NODE_DOWNLOAD: &str = "https://nodejs.org/en/download";

fn npx(package: &str, args: &[&str], env: &[(&str, &str)]) -> Option<ServerTemplate> {
    let mut all = vec![s("-y"), s(package)];
    all.extend(args.iter().map(|a| s(a)));
    Some(ServerTemplate::Stdio {
        command: s("npx"),
        args: all,
        env: env.iter().map(|(k, v)| (s(k), s(v))).collect(),
    })
}

fn bearer(id: &str, url: &str, field: &str) -> Option<ServerTemplate> {
    Some(ServerTemplate::Http {
        url: s(url),
        bearer: Some(variable_for(id, field)),
    })
}

fn secret(id: &str, field: &str, label: &str, placeholder: &str, hint: &str) -> KeyField {
    KeyField {
        key: field.to_ascii_lowercase(),
        label: s(label),
        variable: variable_for(id, field),
        secret: true,
        placeholder: s(placeholder),
        hint: s(hint),
        format: FieldFormat::Any,
        strip_spaces: false,
    }
}

fn prefixed(mut field: KeyField, prefixes: &[&str]) -> KeyField {
    field.format = FieldFormat::Prefix {
        prefixes: lines(prefixes),
    };
    field
}

/// The steps of a sign-in connector.
fn sign_in_steps(app: &str, choose: &str) -> Vec<String> {
    vec![
        format!("Pick the dots that may use {app} below, then click Sign in with {app}."),
        format!("Your browser opens {app}. Sign in if it asks you to."),
        format!("{choose} Then click Allow (or Approve)."),
        s("Come back to Dots: it finishes on its own and checks the connection."),
        format!(
            "To stop, click Disconnect here. You can also remove Dots from {app}'s connected apps."
        ),
    ]
}

#[allow(clippy::too_many_arguments)]
fn oauth(
    id: &str,
    name: &str,
    description: &str,
    category: Category,
    maker: &str,
    url: &str,
    docs: &str,
    choose: &str,
    abilities: &[&str],
) -> Connector {
    Connector {
        id: s(id),
        name: s(name),
        description: s(description),
        category,
        icon: s(id),
        maker: s(maker),
        trust: Trust::Official,
        source_url: s(docs),
        auth: AuthKind::OAuth,
        server: Some(ServerTemplate::Http {
            url: s(url),
            bearer: Some(variable_for(id, "ACCESS_TOKEN")),
        }),
        runtime: None,
        fields: Vec::new(),
        instructions: Instructions {
            steps: sign_in_steps(name, choose),
            open_url: s(docs),
            open_label: format!("Read {maker}'s guide"),
        },
        abilities: lines(abilities),
        note: None,
    }
}

fn coming_soon(id: &str, name: &str, description: &str, category: Category) -> Connector {
    Connector {
        id: s(id),
        name: s(name),
        description: s(description),
        category,
        icon: s(id),
        maker: s("Google"),
        trust: Trust::Official,
        source_url: s("https://developers.google.com/workspace"),
        auth: AuthKind::ComingSoon,
        server: None,
        runtime: None,
        fields: Vec::new(),
        instructions: Instructions {
            steps: lines(&[
                "Not available yet. Google does not let apps register themselves for sign-in the way the other apps here do: Dots needs its own Google sign-in, reviewed by Google, first.",
                "Until then, Gmail works with an app password (see the Gmail connector).",
            ]),
            open_url: s("https://developers.google.com/workspace"),
            open_label: s("About Google Workspace APIs"),
        },
        abilities: Vec::new(),
        note: Some(s("Coming soon: needs Google sign-in for Dots.")),
    }
}

/// Every connector, in the order the screen shows them.
pub fn catalog() -> Vec<Connector> {
    vec![
        Connector {
            id: s("gmail"),
            name: s("Gmail"),
            description: s("Read, search and send email from your Gmail account."),
            category: Category::Communication,
            icon: s("gmail"),
            maker: s("codefuturist"),
            trust: Trust::Community,
            source_url: s("https://github.com/codefuturist/email-mcp"),
            auth: AuthKind::Key,
            server: npx(
                "@codefuturist/email-mcp@0.6.0",
                &["stdio"],
                &[
                    ("MCP_EMAIL_ADDRESS", "${CONNECTOR_GMAIL_ADDRESS}"),
                    ("MCP_EMAIL_PASSWORD", "${CONNECTOR_GMAIL_APP_PASSWORD}"),
                    ("MCP_EMAIL_IMAP_HOST", "imap.gmail.com"),
                    ("MCP_EMAIL_SMTP_HOST", "smtp.gmail.com"),
                ],
            ),
            runtime: node(24),
            fields: vec![
                KeyField {
                    key: s("address"),
                    label: s("Gmail address"),
                    variable: variable_for("gmail", "ADDRESS"),
                    secret: false,
                    placeholder: s("you@gmail.com"),
                    hint: s("The Google account you made the app password in."),
                    format: FieldFormat::Email,
                    strip_spaces: false,
                },
                KeyField {
                    key: s("app_password"),
                    label: s("App password"),
                    variable: variable_for("gmail", "APP_PASSWORD"),
                    secret: true,
                    placeholder: s("abcd efgh ijkl mnop"),
                    hint: s("16 letters. Not your Google password."),
                    format: FieldFormat::Letters { length: 16 },
                    strip_spaces: true,
                },
            ],
            instructions: Instructions {
                steps: lines(&[
                    "App passwords need 2-Step Verification. If it is off, turn it on first at myaccount.google.com/security.",
                    "Click Open Google App passwords below and sign in.",
                    "Type a name such as Dots and click Create.",
                    "Copy the 16-letter password Google shows. It is shown only once.",
                    "Paste it below with your Gmail address. Spaces don't matter.",
                    "To stop, click Disconnect here and delete the app password on the same Google page.",
                ]),
                open_url: s("https://myaccount.google.com/apppasswords"),
                open_label: s("Open Google App passwords"),
            },
            abilities: lines(&[
                "Read and search your mail (IMAP)",
                "Send email as you (SMTP)",
                "Move messages and manage folders and labels",
            ]),
            note: Some(s(
                "Made by codefuturist, not Google. Its program runs on this PC and gets your app password.",
            )),
        },
        Connector {
            id: s("github"),
            name: s("GitHub"),
            description: s("Work with repositories, issues, pull requests and code on GitHub."),
            category: Category::Developer,
            icon: s("github"),
            maker: s("GitHub"),
            trust: Trust::Official,
            source_url: s("https://github.com/github/github-mcp-server"),
            auth: AuthKind::Key,
            server: bearer("github", "https://api.githubcopilot.com/mcp/", "TOKEN"),
            runtime: None,
            fields: vec![prefixed(
                secret(
                    "github",
                    "TOKEN",
                    "Personal access token",
                    "github_pat_…",
                    "Starts with github_pat_ (fine-grained) or ghp_ (classic).",
                ),
                &["github_pat_", "ghp_"],
            )],
            instructions: Instructions {
                steps: lines(&[
                    "Click Open GitHub below. It opens GitHub's New fine-grained token page; sign in if asked.",
                    "Name the token Dots and pick when it expires.",
                    "Under Repository access, choose the repositories your dots may work on.",
                    "Under Permissions, give what they need, for example Contents, Issues and Pull requests: Read and write.",
                    "Click Generate token and copy it. GitHub shows it only once.",
                    "Paste it below. To stop, click Disconnect here and delete the token on GitHub.",
                ]),
                open_url: s("https://github.com/settings/personal-access-tokens/new"),
                open_label: s("Open GitHub"),
            },
            abilities: lines(&[
                "Read code, issues and pull requests in the repositories you allow",
                "Open and comment on issues and pull requests",
                "Search code and repositories",
            ]),
            note: None,
        },
        Connector {
            id: s("slack"),
            name: s("Slack"),
            description: s("Read channels and direct messages and search your Slack workspace."),
            category: Category::Communication,
            icon: s("slack"),
            maker: s("korotovsky"),
            trust: Trust::Community,
            source_url: s("https://github.com/korotovsky/slack-mcp-server"),
            auth: AuthKind::Key,
            server: npx(
                "slack-mcp-server@1.3.0",
                &["--transport", "stdio"],
                &[("SLACK_MCP_XOXP_TOKEN", "${CONNECTOR_SLACK_TOKEN}")],
            ),
            runtime: node(18),
            fields: vec![prefixed(
                secret(
                    "slack",
                    "TOKEN",
                    "User OAuth token",
                    "xoxp-…",
                    "Starts with xoxp-.",
                ),
                &["xoxp-"],
            )],
            instructions: Instructions {
                steps: lines(&[
                    "Click Open Slack apps below and sign in. Click Create New App, then From scratch, name it Dots and pick your workspace.",
                    "Open OAuth & Permissions. Under User Token Scopes add: channels:history, channels:read, groups:history, groups:read, im:history, im:read, mpim:history, mpim:read, users:read and search:read.",
                    "Scroll up and click Install to Workspace, then Allow. Your workspace admin may need to approve it.",
                    "Copy the User OAuth Token (it starts with xoxp-).",
                    "Paste it below. To stop, click Disconnect here and remove the app in Slack.",
                ]),
                open_url: s("https://api.slack.com/apps"),
                open_label: s("Open Slack apps"),
            },
            abilities: lines(&[
                "Read messages in channels and direct messages you can see",
                "Search messages and look up people",
                "Posting messages is off in this connector",
            ]),
            note: Some(s(
                "Made by korotovsky, not Slack. Its program runs on this PC and gets your Slack token. Slack's own server needs an app registered by Dots, which is not available yet.",
            )),
        },
        Connector {
            id: s("brave-search"),
            name: s("Brave Search"),
            description: s("Search the web, news, images and videos with Brave Search."),
            category: Category::Search,
            icon: s("brave-search"),
            maker: s("Brave"),
            trust: Trust::Official,
            source_url: s("https://github.com/brave/brave-search-mcp-server"),
            auth: AuthKind::Key,
            server: npx(
                "@brave/brave-search-mcp-server@2.1.4",
                &["--transport", "stdio"],
                &[("BRAVE_API_KEY", "${CONNECTOR_BRAVE_SEARCH_API_KEY}")],
            ),
            runtime: node(18),
            fields: vec![secret(
                "brave-search",
                "API_KEY",
                "API key",
                "BSA…",
                "From the API Keys page of the Brave Search API dashboard.",
            )],
            instructions: Instructions {
                steps: lines(&[
                    "Click Open Brave Search API below and sign up or sign in.",
                    "Choose a plan. The free plan works for light use.",
                    "Open API Keys and click Add API key.",
                    "Copy the key and paste it below.",
                ]),
                open_url: s("https://api-dashboard.search.brave.com/app/keys"),
                open_label: s("Open Brave Search API"),
            },
            abilities: lines(&[
                "Search the web and news",
                "Find images, videos and local results",
            ]),
            note: None,
        },
        Connector {
            id: s("huggingface"),
            name: s("Hugging Face"),
            description: s("Search models, datasets, Spaces and papers on Hugging Face."),
            category: Category::Developer,
            icon: s("huggingface"),
            maker: s("Hugging Face"),
            trust: Trust::Official,
            source_url: s("https://huggingface.co/docs/hub/en/hf-mcp-server"),
            auth: AuthKind::Key,
            server: bearer("huggingface", "https://huggingface.co/mcp", "TOKEN"),
            runtime: None,
            fields: vec![prefixed(
                secret(
                    "huggingface",
                    "TOKEN",
                    "Access token",
                    "hf_…",
                    "Starts with hf_. A Read token is enough.",
                ),
                &["hf_"],
            )],
            instructions: Instructions {
                steps: lines(&[
                    "Click Open Hugging Face below and sign in.",
                    "Choose the Read token type and name it Dots.",
                    "Click Create token and copy it.",
                    "Paste it below. To stop, click Disconnect here and delete the token on Hugging Face.",
                ]),
                open_url: s("https://huggingface.co/settings/tokens/new?tokenType=read"),
                open_label: s("Open Hugging Face"),
            },
            abilities: lines(&[
                "Search models, datasets and Spaces",
                "Find and read papers and documentation",
            ]),
            note: None,
        },
        oauth(
            "notion",
            "Notion",
            "Search, read and update pages and databases in Notion.",
            Category::Work,
            "Notion",
            "https://mcp.notion.com/mcp",
            "https://developers.notion.com/docs/mcp",
            "Pick the workspace Dots may use.",
            &[
                "Search and read your pages and databases",
                "Create and update pages",
                "Add comments",
            ],
        ),
        oauth(
            "linear",
            "Linear",
            "Find, create and update issues, projects and comments in Linear.",
            Category::Work,
            "Linear",
            "https://mcp.linear.app/mcp",
            "https://linear.app/docs/mcp",
            "Pick the workspace Dots may use.",
            &[
                "Find and read issues and projects",
                "Create and update issues",
                "Add comments",
            ],
        ),
        oauth(
            "atlassian",
            "Jira and Confluence",
            "Work with Jira issues and Confluence pages in your Atlassian site.",
            Category::Work,
            "Atlassian",
            "https://mcp.atlassian.com/v1/mcp",
            "https://support.atlassian.com/rovo/docs/getting-started-with-the-atlassian-remote-mcp-server/",
            "Pick the Atlassian site Dots may use.",
            &[
                "Search, read, create and update Jira issues",
                "Search, read and write Confluence pages",
            ],
        ),
        oauth(
            "sentry",
            "Sentry",
            "Look into errors, issues and releases in Sentry.",
            Category::Developer,
            "Sentry",
            "https://mcp.sentry.dev/mcp",
            "https://docs.sentry.io/product/sentry-mcp/",
            "Pick the organization Dots may use.",
            &[
                "Find and read issues and errors",
                "Look up projects, releases and events",
            ],
        ),
        oauth(
            "todoist",
            "Todoist",
            "Read, add and complete tasks and projects in Todoist.",
            Category::Work,
            "Doist",
            "https://ai.todoist.net/mcp",
            "https://github.com/Doist/todoist-ai",
            "Check what Dots may do.",
            &[
                "Read your tasks and projects",
                "Add, change and complete tasks",
            ],
        ),
        oauth(
            "airtable",
            "Airtable",
            "Read and write records in your Airtable bases.",
            Category::Work,
            "Airtable",
            "https://mcp.airtable.com/mcp",
            "https://support.airtable.com/docs/using-the-airtable-mcp-server",
            "Pick the bases Dots may use.",
            &[
                "Read bases, tables and records",
                "Create and update records",
            ],
        ),
        oauth(
            "stripe",
            "Stripe",
            "Look up customers, payments and invoices in Stripe.",
            Category::Business,
            "Stripe",
            "https://mcp.stripe.com",
            "https://docs.stripe.com/mcp",
            "Pick the Stripe account Dots may use.",
            &[
                "Look up customers, payments and invoices",
                "Create customers, products and payment links",
                "Search Stripe's documentation",
            ],
        ),
        oauth(
            "cloudflare",
            "Cloudflare",
            "Look up and manage your Cloudflare account through its API.",
            Category::Developer,
            "Cloudflare",
            "https://mcp.cloudflare.com/mcp",
            "https://developers.cloudflare.com/agents/model-context-protocol/mcp-servers-for-cloudflare/",
            "Pick the account Dots may use.",
            &[
                "Read your accounts, zones and settings",
                "Call the Cloudflare API on your behalf",
            ],
        ),
        coming_soon(
            "google-calendar",
            "Google Calendar",
            "See and add events in your Google Calendar.",
            Category::Work,
        ),
        coming_soon(
            "google-drive",
            "Google Drive",
            "Find and read files in your Google Drive.",
            Category::Files,
        ),
    ]
}

/// The connector `id`, if the catalog has one.
pub fn find(id: &str) -> Option<Connector> {
    catalog().into_iter().find(|c| c.id == id)
}
