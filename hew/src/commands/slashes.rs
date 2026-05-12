use hew_core::slash;
use hew_core::{Ctx, OutputMode};

pub fn run(ctx: &Ctx, _: ()) -> miette::Result<()> {
    if matches!(ctx.output, OutputMode::Json) {
        let arr: Vec<_> = slash::ALL
            .iter()
            .map(|c| {
                serde_json::json!({
                    "name": c.name,
                    "description": extract_description(c.body).unwrap_or(""),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr).unwrap());
    } else {
        for c in slash::ALL {
            let desc = extract_description(c.body).unwrap_or("");
            println!("  /hew:{:<13} {}", c.name, desc);
        }
    }
    Ok(())
}

/// Pull `description:` from the YAML frontmatter at the top of the body.
fn extract_description(body: &str) -> Option<&str> {
    let lines = body.lines();
    let mut in_frontmatter = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            if in_frontmatter {
                return None;
            }
            in_frontmatter = true;
            continue;
        }
        if in_frontmatter && let Some(rest) = trimmed.strip_prefix("description:") {
            return Some(rest.trim());
        }
    }
    None
}
