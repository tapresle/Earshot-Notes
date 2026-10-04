import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Markdown } from "./Markdown";

function html(source: string): string {
  return render(<Markdown source={source} />).container.firstElementChild!.innerHTML;
}

describe("Markdown", () => {
  it("renders headings one level below the page title", () => {
    expect(html("# Notes\n## Action items")).toBe("<h2>Notes</h2><h3>Action items</h3>");
  });

  it("renders bulleted and numbered lists", () => {
    expect(html("- one\n- two")).toBe("<ul><li>one</li><li>two</li></ul>");
    expect(html("1. first\n2. second")).toBe("<ol><li>first</li><li>second</li></ol>");
  });

  it("nests indented list items", () => {
    expect(html("- parent\n  - child\n- next")).toBe("<ul><li>parent<ul><li>child</li></ul></li><li>next</li></ul>");
  });

  it("joins wrapped lines into one paragraph and separates paragraphs on blank lines", () => {
    expect(html("one\ntwo\n\nthree")).toBe("<p>one two</p><p>three</p>");
  });

  it("renders bold and inline code", () => {
    expect(html("**Dana** owns `budget.xlsx`")).toBe("<p><strong>Dana</strong> owns <code>budget.xlsx</code></p>");
  });

  it("shows HTML from the model as text instead of markup", () => {
    const output = html('<img src=x onerror="alert(1)"> <script>alert(2)</script>');

    expect(output).not.toContain("<img");
    expect(output).not.toContain("<script");
    expect(output).toContain("&lt;img");
  });
});
