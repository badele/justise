// Standard library imports.
use std::fs::File;
use std::io::{self, Write};
use std::process::{Command, ExitCode};

// External crates.
use regex::Regex;
use serde::Deserialize;

//////////////////////////////////////////////////////////////////////////////
// Data models for JSON payloads.
//////////////////////////////////////////////////////////////////////////////
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Task {
    name: String,

    #[serde(default)]
    aliases: Vec<String>,

    #[serde(default)]
    description: String,

    #[serde(default)]
    source: String,

    #[serde(default)]
    depends: Vec<String>,

    #[serde(default)]
    dir: Option<String>,

    #[serde(default)]
    hide: bool,

    #[serde(default)]
    usage: String,
}

#[derive(Debug, Deserialize, Default)]
struct UsageSpecPayload {
    #[serde(default)]
    usage_spec: UsageSpec,
}

#[derive(Debug, Deserialize, Default)]
struct UsageSpec {
    #[serde(default)]
    cmd: UsageCmd,
}

#[derive(Debug, Deserialize, Default)]
struct UsageCmd {
    #[serde(default)]
    usage: String,

    #[serde(default)]
    args: Vec<UsageArg>,

    #[serde(default)]
    flags: Vec<UsageFlag>,
}

#[derive(Debug, Deserialize, Default)]
struct UsageArg {
    #[serde(default)]
    name: String,

    #[serde(default)]
    usage: String,

    #[serde(default)]
    hide: bool,
}

#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)]
struct UsageFlag {
    #[serde(default)]
    name: String,

    #[serde(default)]
    usage: String,

    #[serde(default)]
    short: Vec<String>,

    #[serde(default)]
    long: Vec<String>,

    #[serde(default)]
    hide: bool,

    #[serde(default)]
    arg: Option<UsageArg>,
}

// Render-time annotation for a task.
struct TaskAnnotation {
    task: Task,
    clean_description: String,
    group: String,
    usage_line: String,
    has_usage: bool,
}

// Entry point.
fn main() -> ExitCode {
    // Load tasks via mise CLI and exit on failure.
    let tasks = match read_tasks() {
        Ok(tasks) => tasks,
        Err(err) => {
            eprintln!("failed to read tasks: {err}");
            return ExitCode::FAILURE;
        }
    };

    // Compile the suffix matcher for "Description [Group]".
    let group_suffix_re = match Regex::new(r"^(?s)(.*?)\s*\[([^\[\]]+)\]\s*$") {
        Ok(re) => re,
        Err(err) => {
            eprintln!("failed to compile regex: {err}");
            return ExitCode::FAILURE;
        }
    };

    // Compute render metadata for each task.
    let mut annotated_tasks = Vec::with_capacity(tasks.len());
    let mut max_desc_len = 0;
    for task in tasks {
        if task.name == "justise" {
            continue;
        }

        let (clean_description, group) = split_group(&task.description, &group_suffix_re);
        let has_usage = !task.usage.trim().is_empty();
        let usage_line = if has_usage {
            analyze_command_info(&task.name)
        } else {
            String::new()
        };

        if clean_description.len() > max_desc_len {
            max_desc_len = clean_description.len();
        }

        annotated_tasks.push(TaskAnnotation {
            task,
            clean_description,
            group,
            usage_line,
            has_usage,
        });
    }

    // Open the output file for the generated recipes.
    let file = match File::create("justfile.mise") {
        Ok(file) => file,
        Err(err) => {
            eprintln!("failed to open output: {err}");
            return ExitCode::FAILURE;
        }
    };

    let mut writer = io::BufWriter::new(file);

    // Generate the justfile content and flush it to disk.
    if let Err(err) = write_justfile(&mut writer, &annotated_tasks, max_desc_len) {
        eprintln!("failed to write justfile: {err}");
        return ExitCode::FAILURE;
    }
    if let Err(err) = writer.flush() {
        eprintln!("failed to close output: {err}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

// CLI integration with mise.
fn read_tasks() -> Result<Vec<Task>, Box<dyn std::error::Error>> {
    // Run mise to list tasks as JSON.
    let output = Command::new("mise").args(["task", "ls", "-J"]).output()?;

    // Non-zero exit means the command failed — report stderr only.
    if !output.status.success() {
        let msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if msg.is_empty() {
            "mise task ls -J failed".to_string()
        } else {
            format!("mise task ls -J failed: {msg}")
        };
        return Err(Box::new(io::Error::new(io::ErrorKind::Other, detail)));
    }

    // Decode only stdout — mixing stderr would corrupt the JSON.
    let tasks: Vec<Task> = serde_json::from_slice(&output.stdout)?;
    Ok(tasks)
}

// justfile.mise rendering.
fn write_justfile<W: Write>(
    writer: &mut W,
    tasks: &[TaskAnnotation],
    max_desc_len: usize,
) -> io::Result<()> {
    let mut first = true;

    for annotated_task in tasks {
        // Separate each recipe with a blank line, except before the first one.
        if !first {
            writeln!(writer)?;
        }
        first = false;

        if !annotated_task.task.aliases.is_empty() {
            for alias in &annotated_task.task.aliases {
                writeln!(writer, "alias {alias} := {}", annotated_task.task.name)?;
            }
        }

        let comment_line = render_comment(
            &annotated_task.clean_description,
            &annotated_task.usage_line,
            max_desc_len,
        );
        if !comment_line.is_empty() {
            writeln!(writer, "{comment_line}")?;
        }

        if !annotated_task.group.is_empty() {
            let group_name = escape_single_quotes(&annotated_task.group);
            writeln!(writer, "[group('{group_name}')]")?;
        }

        if annotated_task.task.hide {
            writeln!(writer, "[private]")?;
        }

        if let Some(dir) = &annotated_task.task.dir {
            if !dir.trim().is_empty() {
                let dir_name = escape_single_quotes(dir);
                writeln!(writer, "[working-directory: '{dir_name}']")?;
            }
        }

        if annotated_task.has_usage {
            writeln!(writer, "{} *args:", annotated_task.task.name)?;
            writeln!(writer, "  mise run {} {{args}}", annotated_task.task.name)?;
        } else {
            writeln!(writer, "{}:", annotated_task.task.name)?;
            writeln!(writer, "  mise run {}", annotated_task.task.name)?;
        }
    }

    Ok(())
}

// Helpers.
fn split_group(description: &str, group_suffix_re: &Regex) -> (String, String) {
    // Strip a trailing "[Group]" suffix when present.
    let trimmed = description.trim();
    if let Some(captures) = group_suffix_re.captures(trimmed) {
        let clean = captures.get(1).map(|m| m.as_str().trim()).unwrap_or("");
        let group = captures.get(2).map(|m| m.as_str().trim()).unwrap_or("");
        if group.is_empty() {
            return (trimmed.to_string(), String::new());
        }
        return (clean.to_string(), group.to_string());
    }

    (trimmed.to_string(), String::new())
}

fn analyze_command_info(task_name: &str) -> String {
    // Ask mise for the usage spec of a single task.
    let output = match Command::new("mise")
        .args(["task", task_name, "-J"])
        .output()
    {
        Ok(output) => output,
        // If the command cannot be executed, skip usage.
        Err(_) => return String::new(),
    };

    // Non-zero exit means usage cannot be trusted.
    if !output.status.success() {
        return String::new();
    }

    // Decode only stdout — stderr must not pollute the JSON.
    let payload: UsageSpecPayload = match serde_json::from_slice(&output.stdout) {
        Ok(payload) => payload,
        Err(_) => return String::new(),
    };

    usage_from_payload(task_name, &payload)
}

fn usage_from_payload(task_name: &str, payload: &UsageSpecPayload) -> String {
    let mut usage_parts = Vec::new();

    for flag in &payload.usage_spec.cmd.flags {
        if flag.hide {
            continue;
        }
        let usage = render_flag_usage(flag);
        if !usage.is_empty() {
            usage_parts.push(usage);
        }
    }

    for arg in &payload.usage_spec.cmd.args {
        if arg.hide {
            continue;
        }
        let usage = render_arg_usage(arg);
        if !usage.is_empty() {
            usage_parts.push(usage);
        }
    }

    // Use the command-level usage string as a fallback.
    if usage_parts.is_empty() {
        let fallback = payload.usage_spec.cmd.usage.trim();
        if fallback.is_empty() {
            return String::new();
        }
        return fallback.to_string();
    }

    format!("Usage: {task_name} {}", usage_parts.join(" "))
}

fn render_flag_usage(flag: &UsageFlag) -> String {
    // Prefer explicit usage text, otherwise derive from flag names.
    let usage = flag.usage.trim();
    if !usage.is_empty() {
        return usage.to_string();
    }

    // Use the first long name, or fall back to the first short name.
    let mut name = if let Some(long) = flag.long.first() {
        format!("--{long}")
    } else if let Some(short) = flag.short.first() {
        format!("-{short}")
    } else {
        return String::new();
    };

    if let Some(arg) = &flag.arg {
        // Append argument usage or name when present.
        let arg_usage = if !arg.usage.trim().is_empty() {
            arg.usage.trim().to_string()
        } else if !arg.name.is_empty() {
            format!("<{}>", arg.name)
        } else {
            String::new()
        };

        if !arg_usage.is_empty() {
            name = format!("{name} {arg_usage}");
        }
    }

    name
}

fn render_arg_usage(arg: &UsageArg) -> String {
    // Prefer explicit usage text, otherwise render as <name>.
    let usage = arg.usage.trim();
    if !usage.is_empty() {
        return usage.to_string();
    }
    if arg.name.is_empty() {
        return String::new();
    }
    format!("<{}>", arg.name)
}

fn render_comment(description: &str, usage: &str, max_len: usize) -> String {
    // Align description and usage into a single comment line.
    let desc = description.trim();
    let usage = usage.trim();
    match (desc.is_empty(), usage.is_empty()) {
        (true, true) => String::new(),
        (false, true) => format!("# {desc}"),
        (true, false) => format!("# {usage}"),
        (false, false) => {
            let padding = max_len.saturating_sub(desc.len());
            format!("# {desc}{}  {usage}", " ".repeat(padding))
        }
    }
}

fn escape_single_quotes(value: &str) -> String {
    // Escape single quotes for use inside directives.
    value.replace('\'', "\\'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mise_task_ls_json() {
        let json_tasklist = r#"[
  {
    "name": "optional-and-non-optional-args",
    "aliases": [],
    "description": "Optional and not optinal arguments [args]",
    "source": "mise.toml",
    "depends": [],
    "depends_post": [],
    "wait_for": [],
    "env": [],
    "dir": null,
    "hide": false,
    "global": false,
    "raw": false,
    "interactive": false,
    "sources": [],
    "outputs": [],
    "shell": null,
    "quiet": true,
    "silent": false,
    "tools": {},
    "usage": "arg \"<path>\" help=\"filename\" \narg \"[output]\" help=\"output path\"\n",
    "timeout": null,
    "run": [
      "echo run"
    ],
    "args": [],
    "file": null
  }
]"#;

        let tasks: Vec<Task> = serde_json::from_str(json_tasklist).expect("tasks json parses");
        assert_eq!(tasks.len(), 1);

        let task = &tasks[0];
        assert_eq!(task.name, "optional-and-non-optional-args");
        assert_eq!(
            task.description,
            "Optional and not optinal arguments [args]"
        );
        assert!(task.usage.contains("<path>"));
        assert!(task.usage.contains("[output]"));
    }

    #[test]
    fn usage_from_payload_uses_args() {
        let json_taskinfo = r#"{
  "name": "optional-and-non-optional-args",
  "description": "Optional and not optinal arguments [args]",
  "usage_spec": {
    "cmd": {
      "usage": "<path> [output]",
      "args": [
        { "name": "path", "usage": "<path>", "hide": false },
        { "name": "output", "usage": "[output]", "hide": false }
      ],
      "flags": []
    }
  }
}"#;

        let payload: UsageSpecPayload =
            serde_json::from_str(json_taskinfo).expect("usage json parses");
        let usage = usage_from_payload("optional-and-non-optional-args", &payload);
        assert_eq!(
            usage,
            "Usage: optional-and-non-optional-args <path> [output]"
        );
    }

    #[test]
    fn write_justfile_includes_usage_and_args_marker() {
        let task = Task {
            name: "optional-and-non-optional-args".to_string(),
            aliases: Vec::new(),
            description: "Optional and not optinal arguments [args]".to_string(),
            source: "mise.toml".to_string(),
            depends: Vec::new(),
            dir: None,
            hide: false,
            usage: "arg \"<path>\" help=\"filename\"\narg \"[output]\" help=\"output path\""
                .to_string(),
        };

        let annotated = TaskAnnotation {
            task,
            clean_description: "Optional and not optinal arguments".to_string(),
            group: "args".to_string(),
            usage_line: "Usage: optional-and-non-optional-args <path> [output]".to_string(),
            has_usage: true,
        };

        let annotated_tasks = vec![annotated];
        let mut buffer = Vec::new();
        write_justfile(
            &mut buffer,
            &annotated_tasks,
            "Optional and not optinal arguments".len(),
        )
        .expect("write justfile");

        let output = String::from_utf8(buffer).expect("utf8 output");
        assert!(output.contains("*args:"));
        assert!(output.contains("Usage: optional-and-non-optional-args <path> [output]"));
    }
}
