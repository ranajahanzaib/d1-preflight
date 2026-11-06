use clap::Parser;
use d1_preflight::{
    config::{Cli, Command, Config},
    report,
};
fn main() -> std::process::ExitCode {
    let Cli {
        command: Command::Check(mut o),
    } = Cli::parse();
    let c = match Config::load(&mut o) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Invalid input: {e:#}");
            return 2.into();
        }
    };
    if let Err(e) = d1_preflight::config::validate_outputs(&o) {
        eprintln!("Invalid output: {e:#}");
        return 2.into();
    }
    if o.verbose {
        eprintln!("Building isolated baseline and candidate databases; no remote access.");
    }
    // Validate source paths and output aliases before opening any database.
    let input_paths = o
        .schema
        .iter()
        .chain(o.db.iter())
        .chain(o.data.iter())
        .chain(o.migration.iter())
        .chain(o.queries.iter());
    for p in input_paths {
        if !p.exists() {
            eprintln!("Invalid input: {} does not exist", p.display());
            return 2.into();
        }
    }
    let report = match d1_preflight::analyze(&o, &c) {
        Ok(r) => r,
        Err(e) => {
            let code = if e
                .downcast_ref::<d1_preflight::error::InvalidInput>()
                .is_some()
            {
                2
            } else {
                3
            };
            eprintln!("Analysis failed: {e:#}");
            return code.into();
        }
    };
    let terminal = report.terminal();
    if o.github {
        for line in terminal.lines() {
            println!(" {line}");
        }
    } else {
        println!("{terminal}");
    }
    println!(
        "Policy: {} (threshold {})",
        if report.failed(c.analysis.fail_on) {
            "review required"
        } else {
            "passed within supplied coverage"
        },
        c.analysis.fail_on
    );
    let result = (|| -> anyhow::Result<()> {
        if let Some(p) = &o.json_out {
            d1_preflight::write_output(p, &serde_json::to_string_pretty(&report)?)?;
        }
        if let Some(p) = &o.markdown_out {
            d1_preflight::write_output(p, &report.markdown())?;
        }
        if o.github {
            report::github(&report)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("Report output failed: {e:#}");
        return 3.into();
    }
    u8::from(report.failed(c.analysis.fail_on)).into()
}
