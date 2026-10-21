/**
 * Just enough INI for a form view: settings grouped by section, each with
 * the comment lines above it as help, and a way to change one value without
 * touching anything else in the file.
 */

export interface IniSetting {
  /** Line index in the file. */
  line: number;
  key: string;
  value: string;
  /** The comment lines directly above, without their markers. */
  help: string;
}

export interface IniSection {
  name: string;
  settings: IniSetting[];
}

const isComment = (line: string) => /^\s*[;#]/.test(line);

export function parseIni(text: string): IniSection[] {
  const sections: IniSection[] = [];
  let current: IniSection | null = null;
  let help: string[] = [];
  text.split(/\r?\n/).forEach((raw, line) => {
    const trimmed = raw.trim();
    if (trimmed.startsWith("[") && trimmed.endsWith("]")) {
      current = { name: trimmed.slice(1, -1).trim(), settings: [] };
      sections.push(current);
      help = [];
    } else if (isComment(raw)) {
      help.push(trimmed.replace(/^[;#]+\s?/, ""));
    } else if (trimmed.includes("=")) {
      const eq = trimmed.indexOf("=");
      if (!current) {
        current = { name: "", settings: [] };
        sections.push(current);
      }
      current.settings.push({
        line,
        key: trimmed.slice(0, eq).trim(),
        value: trimmed.slice(eq + 1).trim(),
        help: help.join("\n"),
      });
      help = [];
    } else {
      help = [];
    }
  });
  return sections.filter((section) => section.settings.length > 0);
}

/** `text` with the value on `line` replaced, keeping the key, spacing and line endings. */
export function setIniValue(text: string, line: number, value: string): string {
  const newline = text.includes("\r\n") ? "\r\n" : "\n";
  const lines = text.split(/\r?\n/);
  const raw = lines[line];
  if (raw === undefined) return text;
  const eq = raw.indexOf("=");
  if (eq === -1) return text;
  const after = raw.slice(eq + 1);
  const gap = after.slice(0, after.length - after.trimStart().length);
  lines[line] = `${raw.slice(0, eq)}=${gap}${value}`;
  return lines.join(newline);
}
