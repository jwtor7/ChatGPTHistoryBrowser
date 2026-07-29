use chatgpt_history_browser::structure_inspector;

fn main() {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    let exit_code =
        structure_inspector::run_cli(std::env::args_os().skip(1), &mut stdin, &mut stdout);
    std::process::exit(exit_code);
}
