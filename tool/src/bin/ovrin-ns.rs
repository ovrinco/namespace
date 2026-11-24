fn main() {
    match ovrin_ns::cli::run() {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!(
                "{}",
                serde_json::json!({"error":"namespace_error","detail":e.to_string()})
            );
            std::process::exit(2);
        }
    }
}
