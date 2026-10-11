import { Fragment, type ReactNode } from "react";

/**
 * Renders a citation group, such as the labels `S3` and `N1` from `[S3, N1]`.
 * Returning null drops the group.
 */
export type RenderCitation = (labels: string[]) => ReactNode;

/**
 * Renders the Markdown subset summaries and chat answers use: headings,
 * bulleted and numbered lists (nested by indentation), paragraphs, bold, and
 * inline code. It builds React elements and never injects HTML, so model
 * output can't inject markup. With `citation`, groups such as `[S3, N1]` are
 * rendered by it instead of as text.
 */
export function Markdown({ source, citation }: { source: string; citation?: RenderCitation }) {
  return <div className="markdown">{renderBlocks(source, citation)}</div>;
}

type Block =
  | { kind: "heading"; level: number; text: string }
  | { kind: "list"; ordered: boolean; items: ListItem[] }
  | { kind: "paragraph"; text: string };

interface ListItem {
  text: string;
  children: Block[];
}

const listMarker = /^(\s*)([-*+]|\d+[.)])\s+(.*)$/;

function parse(lines: string[]): Block[] {
  const blocks: Block[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) {
      i++;
      continue;
    }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line.trim());
    if (heading) {
      blocks.push({ kind: "heading", level: heading[1].length, text: heading[2] });
      i++;
      continue;
    }
    const marker = listMarker.exec(line);
    if (marker) {
      const [list, next] = parseList(lines, i, marker[1].length);
      blocks.push(list);
      i = next;
      continue;
    }
    const paragraph: string[] = [];
    while (i < lines.length && lines[i].trim() && !listMarker.test(lines[i]) && !/^#{1,6}\s/.test(lines[i].trim())) {
      paragraph.push(lines[i].trim());
      i++;
    }
    blocks.push({ kind: "paragraph", text: paragraph.join(" ") });
  }
  return blocks;
}

function parseList(lines: string[], start: number, indent: number): [Block, number] {
  const ordered = /\d/.test(listMarker.exec(lines[start])![2]);
  const items: ListItem[] = [];
  let i = start;
  while (i < lines.length) {
    const marker = listMarker.exec(lines[i]);
    if (!lines[i].trim()) {
      i++;
      continue;
    }
    if (!marker || marker[1].length < indent) break;
    if (marker[1].length > indent && items.length > 0) {
      const [child, next] = parseList(lines, i, marker[1].length);
      items[items.length - 1].children.push(child);
      i = next;
      continue;
    }
    items.push({ text: marker[3], children: [] });
    i++;
  }
  return [{ kind: "list", ordered, items }, i];
}

function renderBlocks(source: string, citation?: RenderCitation): ReactNode {
  return parse(source.split("\n")).map((block, key) => renderBlock(block, key, citation));
}

function renderBlock(block: Block, key: number, citation?: RenderCitation): ReactNode {
  switch (block.kind) {
    case "heading": {
      // The meeting title is the page's h1, so summary headings start at h2.
      const level = Math.min(block.level + 1, 6);
      const Tag = `h${level}` as "h2";
      return <Tag key={key}>{inline(block.text, citation)}</Tag>;
    }
    case "paragraph":
      return <p key={key}>{inline(block.text, citation)}</p>;
    case "list": {
      const Tag = block.ordered ? "ol" : "ul";
      return (
        <Tag key={key}>
          {block.items.map((item, i) => (
            <li key={i}>
              {inline(item.text, citation)}
              {item.children.map((child, i) => renderBlock(child, i, citation))}
            </li>
          ))}
        </Tag>
      );
    }
  }
}

/** ASCII punctuation, which a backslash escapes in Markdown. */
const ESCAPABLE = /[!-/:-@[-`{-~]/;

/** A citation group such as `[S3]` or `[S3, N1]`, matching `chat/citations.rs`. */
const CITATION = /^\[([SN]\d+(?:\s*[,;]\s*[SN]\d+)*)\]/;

/**
 * Bold (`**text**`), inline code (`` `text` ``), and backslash escapes such as
 * `\$48,000`; everything else is plain text. A backslash before punctuation is
 * dropped, except inside code, so escaped asterisks are never bold markers.
 */
function inline(text: string, citation?: RenderCitation): ReactNode {
  const nodes: ReactNode[] = [];
  let plain = "";
  const flush = () => {
    if (plain) nodes.push(<Fragment key={nodes.length}>{plain}</Fragment>);
    plain = "";
  };
  let i = 0;
  while (i < text.length) {
    const rest = text.slice(i);
    if (rest[0] === "\\" && rest.length > 1 && ESCAPABLE.test(rest[1])) {
      plain += rest[1];
      i += 2;
      continue;
    }
    const code = /^`([^`]+)`/.exec(rest);
    if (code) {
      flush();
      nodes.push(<code key={nodes.length}>{code[1]}</code>);
      i += code[0].length;
      continue;
    }
    const cited = citation && CITATION.exec(rest);
    if (cited) {
      const rendered = citation(cited[1].split(/[,;]/).map((label) => label.trim()));
      if (rendered == null) {
        plain = plain.trimEnd(); // A dropped group takes the space before it.
      } else {
        flush();
        nodes.push(<Fragment key={nodes.length}>{rendered}</Fragment>);
      }
      i += cited[0].length;
      continue;
    }
    const bold = /^\*\*((?:\\.|[^*\\])+)\*\*/.exec(rest);
    if (bold) {
      flush();
      nodes.push(<strong key={nodes.length}>{inline(bold[1], citation)}</strong>);
      i += bold[0].length;
      continue;
    }
    plain += rest[0];
    i += 1;
  }
  flush();
  return nodes;
}
