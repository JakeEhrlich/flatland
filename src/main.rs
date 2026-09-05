fn main() {
    // Behave like a normal Unix tool when stdout is closed early (`pcb status | head`):
    // die quietly on SIGPIPE instead of panicking.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    // miette renders our diagnostics with source snippets and help text.
    miette::set_hook(Box::new(|_| {
        Box::new(miette::MietteHandlerOpts::new().terminal_links(false).width(100).build())
    }))
    .ok();
    if let Err(e) = flatland::cli::run() {
        eprintln!("{:?}", miette::Report::new(e));
        std::process::exit(1);
    }
}
