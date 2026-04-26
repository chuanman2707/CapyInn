use capyinn_lib::restore_drill::{run_restore_drill, RestoreDrillOptions};
use std::{env, path::PathBuf};

#[tokio::main]
async fn main() {
    let options = match parse_args(env::args().skip(1)) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };

    let result = run_restore_drill(options).await;
    println!("Status: {}", result.status.as_str());
    match &result.report_path {
        Some(path) => println!("Report: {}", path.display()),
        None => eprintln!("Report: not written"),
    }
    if result.status.as_str() == "FAIL" {
        eprintln!("{}", result.message);
    }

    std::process::exit(result.exit_code());
}

fn parse_args<I>(args: I) -> Result<RestoreDrillOptions, String>
where
    I: IntoIterator<Item = String>,
{
    let mut options = RestoreDrillOptions::default();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--runtime-root" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--runtime-root requires a path".to_string())?;
                options.runtime_root = Some(PathBuf::from(value));
            }
            "--backup" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--backup requires a path".to_string())?;
                options.backup_path = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                return Err(
                    "Usage: restore_drill [--runtime-root <path>] [--backup <path>]".to_string(),
                );
            }
            unknown => return Err(format!("unknown argument: {unknown}")),
        }
    }

    Ok(options)
}
