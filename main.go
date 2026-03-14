package main

import (
	"bufio"
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/exec"
	"regexp"
	"strings"
)

type Task struct {
	Name        string   `json:"name"`
	Aliases     []string `json:"aliases"`
	Description string   `json:"description"`
	Source      string   `json:"source"`
	Depends     []string `json:"depends"`
	Dir         *string  `json:"dir"`
	Hide        bool     `json:"hide"`
	Usage       string   `json:"usage"`
}

type RenderTask struct {
	Task
	CleanDescription string
	Group            string
	UsageLine        string
	HasUsage         bool
}

var (
	groupSuffixRe = regexp.MustCompile(`^(?s)(.*?)\s*\[([^\[\]]+)\]\s*$`)
)

func main() {
	// Load tasks from mise JSON output.
	tasks, err := readTasks()
	if err != nil {
		fmt.Fprintf(os.Stderr, "failed to read tasks: %v\n", err)
		os.Exit(1)
	}

	// Normalize descriptions, group tags, and usage lines for rendering.
	renderTasks := make([]RenderTask, 0, len(tasks))
	maxDescLen := 0
	for _, task := range tasks {
		if task.Name == "justise" {
			continue
		}

		cleanDesc, group := splitGroup(task.Description)
		hasUsage := strings.TrimSpace(task.Usage) != ""
		usageLine := ""
		if hasUsage {
			usageLine = fetchUsageLine(task.Name)
		}
		renderTask := RenderTask{
			Task:             task,
			CleanDescription: cleanDesc,
			Group:            group,
			UsageLine:        usageLine,
			HasUsage:         hasUsage,
		}
		renderTasks = append(renderTasks, renderTask)
		if len(cleanDesc) > maxDescLen {
			maxDescLen = len(cleanDesc)
		}
	}

	// Write the generated Justfile wrapper.
	outputFile, err := os.Create("justfile.mise")
	if err != nil {
		fmt.Fprintf(os.Stderr, "failed to open output: %v\n", err)
		os.Exit(1)
	}
	if err := writeJustfile(outputFile, renderTasks, maxDescLen); err != nil {
		fmt.Fprintf(os.Stderr, "failed to write justfile: %v\n", err)
		os.Exit(1)
	}
	if err := outputFile.Close(); err != nil {
		fmt.Fprintf(os.Stderr, "failed to close output: %v\n", err)
		os.Exit(1)
	}
}

func readTasks() ([]Task, error) {
	// Call mise directly to obtain JSON tasks.
	cmd := exec.Command("mise", "task", "ls", "-J")
	cmd.Env = append(os.Environ(), "NO_COLOR=1", "CLICOLOR=0")
	output, err := cmd.CombinedOutput()
	if err != nil {
		trimmed := strings.TrimSpace(string(output))
		if trimmed == "" {
			return nil, err
		}
		return nil, fmt.Errorf("mise task ls -J failed: %s", trimmed)
	}

	decoder := json.NewDecoder(bytes.NewReader(output))
	var tasks []Task
	if err := decoder.Decode(&tasks); err != nil {
		return nil, err
	}

	return tasks, nil
}

// Export mise tasks to just recipes
func writeJustfile(writer io.Writer, tasks []RenderTask, maxDescLen int) error {
	// Render tasks in the same order as the JSON payload.
	buffered := bufio.NewWriter(writer)
	for index, task := range tasks {
		if index > 0 {
			if _, err := fmt.Fprintln(buffered); err != nil {
				return err
			}
		}

		// Aliases belong immediately above the recipe.
		if len(task.Aliases) > 0 {
			for _, alias := range task.Aliases {
				if _, err := fmt.Fprintf(buffered, "alias %s := %s\n", alias, task.Name); err != nil {
					return err
				}
			}
		}

		// Single-line comment with aligned description + usage.
		commentLine := renderComment(task.CleanDescription, task.UsageLine, maxDescLen)
		if commentLine != "" {
			if _, err := fmt.Fprintln(buffered, commentLine); err != nil {
				return err
			}
		}

		// Directives must appear after comments.
		if task.Group != "" {
			groupName := escapeSingleQuotes(task.Group)
			if _, err := fmt.Fprintf(buffered, "[group('%s')]\n", groupName); err != nil {
				return err
			}
		}

		if task.Hide {
			if _, err := fmt.Fprintln(buffered, "[private]"); err != nil {
				return err
			}
		}

		if task.Dir != nil && strings.TrimSpace(*task.Dir) != "" {
			dirName := escapeSingleQuotes(*task.Dir)
			if _, err := fmt.Fprintf(buffered, "[working-directory: '%s']\n", dirName); err != nil {
				return err
			}
		}

		if task.HasUsage {
			if _, err := fmt.Fprintf(buffered, "%s *args:\n", task.Name); err != nil {
				return err
			}
			if _, err := fmt.Fprintf(buffered, "  mise run %s {{args}}\n", task.Name); err != nil {
				return err
			}
		} else {
			if _, err := fmt.Fprintf(buffered, "%s:\n", task.Name); err != nil {
				return err
			}
			if _, err := fmt.Fprintf(buffered, "  mise run %s\n", task.Name); err != nil {
				return err
			}
		}
	}

	return buffered.Flush()
}

func splitGroup(description string) (string, string) {
	// Detect a trailing "[Group]" suffix and strip it.
	trimmed := strings.TrimSpace(description)
	matches := groupSuffixRe.FindStringSubmatch(trimmed)
	if len(matches) != 3 {
		return trimmed, ""
	}

	clean := strings.TrimSpace(matches[1])
	group := strings.TrimSpace(matches[2])
	if group == "" {
		return trimmed, ""
	}

	return clean, group
}

func fetchUsageLine(taskName string) string {
	// Query mise help output for the usage line.
	cmd := exec.Command("mise", "run", taskName, "-h")
	cmd.Env = append(os.Environ(), "NO_COLOR=1", "CLICOLOR=0")
	output, err := cmd.CombinedOutput()
	if err != nil {
		return ""
	}

	for _, line := range strings.Split(string(output), "\n") {
		trimmed := strings.TrimSpace(line)
		if strings.HasPrefix(trimmed, "Usage:") {
			return trimmed
		}
	}

	return ""
}

func renderComment(description, usage string, maxLen int) string {
	// Combine description and usage on one aligned line.
	desc := strings.TrimSpace(description)
	usage = strings.TrimSpace(usage)
	if desc == "" && usage == "" {
		return ""
	}
	if usage == "" {
		return "# " + desc
	}
	if desc == "" {
		return "# " + usage
	}

	padding := maxLen - len(desc)
	if padding < 0 {
		padding = 0
	}
	return "# " + desc + strings.Repeat(" ", padding) + "  " + usage
}

func escapeSingleQuotes(value string) string {
	// Ensure single quotes in directives are escaped.
	return strings.ReplaceAll(value, "'", "\\'")
}
