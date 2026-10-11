import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
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

  it("drops the backslash from escaped punctuation", () => {
    expect(html("Harbor Street costs \\$48,000 a year")).toBe("<p>Harbor Street costs $48,000 a year</p>");
    expect(html("\\*\\*not bold\\*\\*")).toBe("<p>**not bold**</p>");
    expect(html("**\\$5** each")).toBe("<p><strong>$5</strong> each</p>");
  });

  it("keeps escaped block markers as text", () => {
    expect(html("\\# not a heading")).toBe("<p># not a heading</p>");
    expect(html("\\- not a list")).toBe("<p>- not a list</p>");
  });

  it("keeps a backslash before anything else, and inside code", () => {
    expect(html("Saved to C:\\Users\\dana")).toBe("<p>Saved to C:\\Users\\dana</p>");
    expect(html("Run `grep \\$HOME`")).toBe("<p>Run <code>grep \\$HOME</code></p>");
    expect(html("ends with \\")).toBe("<p>ends with \\</p>");
  });

  it("shows HTML from the model as text instead of markup", () => {
    const output = html('<img src=x onerror="alert(1)"> <script>alert(2)</script>');

    expect(output).not.toContain("<img");
    expect(output).not.toContain("<script");
    expect(output).toContain("&lt;img");
  });

  it("leaves citation labels as text without a citation renderer", () => {
    expect(html("Friday [S2].")).toBe("<p>Friday [S2].</p>");
  });

  it("renders citation groups with the citation renderer", () => {
    const cite = (labels: string[]) => <sup>{labels.join("+")}</sup>;
    const out = render(<Markdown source={"- Friday [S2, N1] and **May [S3]** [x] \\[S4]"} citation={cite} />);

    expect(out.container.firstElementChild!.innerHTML).toBe(
      "<ul><li>Friday <sup>S2+N1</sup> and <strong>May <sup>S3</sup></strong> [x] [S4]</li></ul>",
    );
  });

  it("gives a citation a key of its own after the text before it", () => {
    const errors = vi.spyOn(console, "error").mockImplementation(() => {});
    const cite = (labels: string[]) => <sup>{labels.join("+")}</sup>;

    render(<Markdown source="Friday [S2] and Monday [S3]." citation={cite} />);
    const logged = errors.mock.calls.length;
    errors.mockRestore();

    expect(logged).toBe(0);
  });

  it("drops a citation group the renderer returns nothing for, with the space before it", () => {
    const out = render(<Markdown source="Friday [S9]. Done [S1]." citation={() => null} />);

    expect(out.container.firstElementChild!.innerHTML).toBe("<p>Friday. Done.</p>");
  });
});
