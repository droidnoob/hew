use clap::Args as ClapArgs;
use hew_core::skills::{self, Category};
use hew_core::{Ctx, OutputMode};

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Filter by category: core, brownfield, optional, or index.
    #[arg(long, value_enum)]
    pub category: Option<CategoryArg>,
}

#[derive(Debug, Copy, Clone, clap::ValueEnum, PartialEq, Eq)]
pub enum CategoryArg {
    Core,
    Brownfield,
    Optional,
    Index,
}

fn matches(c: Category, want: Option<CategoryArg>) -> bool {
    match want {
        None => true,
        Some(CategoryArg::Core) => c == Category::Core,
        Some(CategoryArg::Brownfield) => c == Category::Brownfield,
        Some(CategoryArg::Optional) => c == Category::Optional,
        Some(CategoryArg::Index) => c == Category::Index,
    }
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    let filtered: Vec<_> =
        skills::all().into_iter().filter(|s| matches(s.category, args.category)).collect();

    if matches!(ctx.output, OutputMode::Json) {
        let arr: Vec<_> = filtered
            .iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "category": s.category.to_string(),
                    "path": s.relative_path,
                    "version": s.version().unwrap_or("?"),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr).unwrap());
    } else {
        for s in &filtered {
            println!("  {:<11} {:<18} {}", s.category, s.name, s.relative_path);
        }
    }
    Ok(())
}
