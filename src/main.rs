use anyhow::Result;
use clap::Parser;
use log::{info, trace};
use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;
use std::{
    collections::HashSet,
    fs::File,
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Parser)]
#[clap(author,version,about,long_about=None)]
struct Cli {
    path: PathBuf,

    #[clap(flatten)]
    verbose: clap_verbosity_flag::Verbosity,

    #[clap(short, long)]
    watch: bool,

    #[clap(short, long, value_parser)]
    output: Option<PathBuf>,
}

async fn debounce_watch<P1: AsRef<Path>, P2: AsRef<Path>>(
    path: P1,
    output_path: P2,
    dictionary: &mut HashSet<String>,
    stdout_handle: &mut impl Write,
) {
    let (tx, rx) = std::sync::mpsc::channel();

    let mut debouncer = new_debouncer(Duration::from_millis(250), None, tx).unwrap();

    debouncer
        .watcher()
        .watch(path.as_ref(), RecursiveMode::NonRecursive)
        .unwrap();

    for events in rx {
        match events {
            Ok(event) => {
                trace!("{:?}", event);

                // Editor may temporarily rename the input file while saving it
                if markwrite::update_html(&path, &output_path, dictionary, stdout_handle)
                    .await
                    .is_err()
                {
                    info!("[ INFO ] Looks like the input file was renamed.");
                };
            }
            Err(e) => eprintln!("[ ERROR ] watch error: {:?}.", e),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = &Cli::parse();
    env_logger::Builder::new()
        .filter_level(cli.verbose.log_level_filter())
        .init();
    let path = &cli.path;
    let mut default_output_path = PathBuf::from(path);
    default_output_path.set_extension("html");
    let output_path = match &cli.output {
        Some(value) => value,
        None => &default_output_path,
    };

    /* Check input file exists. Do the check here, rather than handle on each
     * modification since, text editor may temporarily rename the original file
     * on saving it.
     */
    if File::open(path).is_err() {
        let error_message = match path.to_str() {
            Some(value) => {
                format!("[ ERROR ] Unable to open input ({value}), check the path is correct.")
            }
            None => "[ ERROR ] Unable to open input, check the path is correct.".to_string(),
        };
        return Err(error_message.into());
    }

    let stdout = io::stdout();
    let mut stdout_handle = io::BufWriter::new(stdout);
    let mut dictionary: HashSet<String> = HashSet::new();
    markwrite::load_dictionary(
        ".markwrite/custom.dict",
        &mut dictionary,
        &mut stdout_handle,
    );
    // Watch for input file modifications and generate HTML when they occur.
    writeln!(stdout_handle, "[ INFO ] waiting for file changes.")?;
    stdout_handle.flush()?;

    debounce_watch(path, output_path, &mut dictionary, &mut stdout_handle).await;
    Ok(())
}
