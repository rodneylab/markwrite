use anyhow::Result;
use clap::Parser;
use futures::{
    channel::mpsc::{channel, Receiver},
    SinkExt, StreamExt,
};
use log::info;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    fs::File,
    io::{self, Write},
    path::{Path, PathBuf},
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = &Cli::parse();
    env_logger::Builder::new()
        .filter_level(cli.verbose.log_level_filter())
        .init();
    let path = &cli.path;

    // set output path to provided value or set to default
    let default_output_path = match path.file_stem() {
        Some(stem_value) => {
            let mut stem = PathBuf::from(stem_value);
            stem.set_extension("html");
            stem
        }
        None => PathBuf::from("unnamed.html"),
    };
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

    // Watch for input file modifications and generate HTML when they occur.
    let stdout = io::stdout();
    let mut stdout_handle = io::BufWriter::new(stdout);
    writeln!(stdout_handle, "[ INFO ] waiting for file changes.")?;
    stdout_handle.flush()?;
    futures::executor::block_on(async {
        if let Err(e) = async_watch(path, output_path).await {
            eprintln!(
                "[ ERROR ] Somthing went wrong in setup for watching the input: {:?}",
                e
            )
        }
    });

    Ok(())
}

fn async_watcher() -> notify::Result<(RecommendedWatcher, Receiver<notify::Result<Event>>)> {
    let (mut tx, rx) = channel(1);

    let watcher = RecommendedWatcher::new(
        move |res| {
            futures::executor::block_on(async {
                tx.send(res).await.unwrap();
            })
        },
        Config::default(),
    )?;

    Ok((watcher, rx))
}

async fn async_watch<P1: AsRef<Path>, P2: AsRef<Path>>(
    path: P1,
    output_path: P2,
) -> notify::Result<()> {
    let (mut watcher, mut rx) = async_watcher()?;

    watcher.watch(path.as_ref(), RecursiveMode::NonRecursive)?;

    while let Some(res) = rx.next().await {
        match res {
            Ok(event) => {
                info!("[ INFO ] Input file change: {:?}", event);

                // Editor may temporarily rename the input file while saving it
                if markwrite::update_html(&path, &output_path).is_err() {
                    info!("[ INFO ] Looks like the input file was renamed.");
                };
            }
            Err(e) => eprintln!("[ ERROR ] watch error: {:?}.", e),
        }
    }

    Ok(())
}
