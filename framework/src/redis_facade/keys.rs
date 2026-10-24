//! Key positions of the typed commands. Raw commands never call this helper.

pub(crate) fn prefixed(command: &str, args: &[&str], prefix: &str) -> Vec<String> {
    args.iter()
        .enumerate()
        .map(|(index, arg)| {
            let key = match command {
                "DEL" | "MGET" => true,
                "BLPOP" | "BRPOP" | "BZPOPMIN" | "BZPOPMAX" => index + 1 < args.len(),
                "BLMOVE" | "BRPOPLPUSH" => index < 2,
                "SCAN" => index == 2,
                "PUBLISH" => false,
                _ => index == 0,
            };
            if command == "SCAN" && key {
                let mut pattern = String::with_capacity(prefix.len() + arg.len());
                for character in prefix.chars() {
                    if matches!(character, '*' | '?' | '[' | ']' | '\\') {
                        pattern.push('\\');
                    }
                    pattern.push(character);
                }
                pattern.push_str(arg);
                pattern
            } else if key {
                format!("{prefix}{arg}")
            } else {
                (*arg).to_owned()
            }
        })
        .collect()
}
