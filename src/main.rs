use anyhow::{Context as AnyhowContext, Result};
use handlebars::Handlebars;
use log::error;
use mdbook::book::{Book, BookItem};
use mdbook::errors::Error;
use mdbook::preprocess::{CmdPreprocessor, Preprocessor, PreprocessorContext};
use regex::Regex;
use serde_json::Value as Json;
use std::fs;
use std::io;
use std::process;

struct Template;

impl Preprocessor for Template {
    fn name(&self) -> &str {
        "template"
    }

    fn run(&self, ctx: &PreprocessorContext, mut book: Book) -> std::result::Result<Book, Error> {
        // Read config: [preprocessor.template] paths = ["docs/book/assets/operators.json", "docs/book/assets/portals.json"]
        let cfg = ctx
            .config
            .get("preprocessor.template")
            .and_then(|v| v.as_table())
            .ok_or_else(|| Error::msg("missing [preprocessor.template] config with a `paths` array"))?;

        let paths = cfg
            .get("paths")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error::msg("missing `paths` array under [preprocessor.template]"))?;

        // Merge all JSON files into a single template context (shallow merge on top-level keys).
        let mut context = Json::Object(serde_json::Map::new());
        for p in paths {
            let path = p
                .as_str()
                .ok_or_else(|| Error::msg("paths entries must be strings"))?;
            let txt = fs::read_to_string(path)
                .with_context(|| format!("reading {}", path))
                .map_err(Error::from)?;
            let val: Json = serde_json::from_str(&txt)
                .with_context(|| format!("parsing {}", path))
                .map_err(Error::from)?;

            if let Json::Object(map) = val {
                if let Json::Object(ctx_map) = &mut context {
                    for (k, v) in map {
                        ctx_map.insert(k, v);
                    }
                }
            }
        }

        // Render every chapter with Handlebars. Chapters without template tags pass through unchanged.
        let hbs = Handlebars::new();
        // Regex to match ${{ ... }} patterns
        let dollar_brace_re = Regex::new(r"\$\{\{.*?\}\}").unwrap();

        book.for_each_mut(|item| {
            if let BookItem::Chapter(ch) = item {
                // Store all ${{ ... }} patterns and replace with placeholders
                let mut protected_patterns = Vec::new();
                let mut placeholder_idx = 0;

                let protected_content = dollar_brace_re.replace_all(&ch.content, |caps: &regex::Captures| {
                    let full_match = caps.get(0).unwrap().as_str();
                    protected_patterns.push(full_match.to_string());
                    let placeholder = format!("__PROTECTED_PATTERN_{}__", placeholder_idx);
                    placeholder_idx += 1;
                    placeholder
                });

                match hbs.render_template(&protected_content, &context) {
                    Ok(mut rendered) => {
                        // Restore all protected patterns
                        for (idx, pattern) in protected_patterns.iter().enumerate() {
                            let placeholder = format!("__PROTECTED_PATTERN_{}__", idx);
                            rendered = rendered.replace(&placeholder, pattern);
                        }
                        ch.content = rendered;
                    },
                    Err(e) => error!("Handlebars render error in {}: {}", ch.name, e),
                }
            }
        });

        Ok(book)
    }

    fn supports_renderer(&self, _renderer: &str) -> bool {
        true
    }
}

fn main() -> Result<()> {
    env_logger::init();

    let template = Template;
    let mut args = std::env::args();
    let _prog = args.next();

    if let Some(cmd) = args.next() {
        if cmd == "supports" {
            let renderer = args.next().unwrap_or_default();
            if template.supports_renderer(&renderer) {
                process::exit(0);
            } else {
                process::exit(1);
            }
        }
    }

    // Parse input (context + book) from stdin using mdBook's helper.
    let (ctx, book) = CmdPreprocessor::parse_input(io::stdin()).map_err(|e| anyhow::anyhow!(e))?;

    // Run the preprocessor and write the processed book to stdout.
    let processed = template.run(&ctx, book).map_err(|e| anyhow::anyhow!(e))?;
    serde_json::to_writer(io::stdout(), &processed).context("writing processed book to stdout")?;
    Ok(())
}
