//! Bash code and data boundary. Templates are static, values use positional arguments.
pub const BASH_STDIN: &str = "bash --noprofile --norc -s --";

pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn script(body: &str, args: &[&str]) -> Result<String, String> {
    if body.contains('\0') || args.iter().any(|arg| arg.contains('\0')) {
        return Err("执行参数包含无效字符，请检查部署配置".to_string());
    }
    let args = args
        .iter()
        .map(|arg| quote(arg))
        .collect::<Vec<_>>()
        .join(" ");
    Ok(format!("set -euo pipefail\nset -- {args}\n{body}\n"))
}

/// Bulk data owns stdin, so the static transfer program is passed via -c.
pub fn command(script: &str) -> String {
    format!("bash --noprofile --norc -c {}", quote(script))
}

/// Bash parses the entire compound command before any child can consume stdin.
pub fn stdin_program(script: &str) -> Vec<u8> {
    format!("{{\n{script}\n}}\n").into_bytes()
}

/// Execute an existing script with argv; never invoke a local shell to join arguments.
pub fn file_command(binary: &str, path: &str) -> std::process::Command {
    let mut command = std::process::Command::new(binary);
    command.args(["--noprofile", "--norc", "--", path]);
    command
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::core::process::{self, Input, Options};
    #[test]
    fn positional_values_are_literal_and_children_cannot_consume_script() {
        let values = ["", "a'b", "$(exit 7)", "`exit 8`", "a\nb", "-n"];
        let body = script("cat >/dev/null\nprintf '%s\\0' \"$@\"", &values).unwrap();
        let mut command = std::process::Command::new("bash");
        command.args(["--noprofile", "--norc", "-s", "--"]);
        let output = process::run(
            command,
            Input::Bytes(stdin_program(&body)),
            Options::default(),
        )
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut expected = values.join("\0").into_bytes();
        expected.push(0);
        assert_eq!(output.stdout, expected);
        assert!(script("true", &["bad\0value"]).is_err());
    }
}
