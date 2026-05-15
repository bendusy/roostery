fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 && (args[1] == "--version" || args[1] == "-V") {
        println!("roostery {} (rust)", roostery::VERSION);
        return;
    }
    println!(
        "roostery {} (rust) — see https://github.com/bendusy/roostery",
        roostery::VERSION
    );
}
