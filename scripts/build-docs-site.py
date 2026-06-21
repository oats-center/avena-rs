#!/usr/bin/env python3
import argparse
import html
import re
import shutil
from pathlib import Path


DOCS = [
    ("Setup Guide", "docs/setup-guide.md", "docs/setup-guide/index.html"),
    ("Schematics", "schematics/README.md", "docs/schematics/index.html"),
]


def slugify(text: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")
    return slug or "section"


def inline_markdown(text: str) -> str:
    escaped = html.escape(text)
    escaped = re.sub(r"`([^`]+)`", r"<code>\1</code>", escaped)
    escaped = re.sub(r"\*\*([^*]+)\*\*", r"<strong>\1</strong>", escaped)
    escaped = re.sub(
        r"\[([^\]]+)\]\(([^)]+)\)",
        lambda match: f'<a href="{html.escape(match.group(2), quote=True)}">{match.group(1)}</a>',
        escaped,
    )
    return escaped


def close_lists(out: list[str], list_stack: list[str]) -> None:
    while list_stack:
        out.append(f"</{list_stack.pop()}>")


def render_markdown(markdown: str) -> tuple[str, list[tuple[int, str, str]]]:
    out: list[str] = []
    headings: list[tuple[int, str, str]] = []
    list_stack: list[str] = []
    in_code = False
    code_lines: list[str] = []

    for raw_line in markdown.splitlines():
        line = raw_line.rstrip()

        if line.startswith("```"):
            if in_code:
                out.append("<pre><code>")
                out.append(html.escape("\n".join(code_lines)))
                out.append("</code></pre>")
                code_lines = []
                in_code = False
            else:
                close_lists(out, list_stack)
                in_code = True
            continue

        if in_code:
            code_lines.append(raw_line)
            continue

        if not line.strip():
            close_lists(out, list_stack)
            continue

        heading = re.match(r"^(#{1,4})\s+(.+)$", line)
        if heading:
            close_lists(out, list_stack)
            level = len(heading.group(1))
            title = heading.group(2).strip()
            slug = slugify(title)
            headings.append((level, title, slug))
            out.append(f'<h{level} id="{slug}">{inline_markdown(title)}</h{level}>')
            continue

        bullet = re.match(r"^\s*[-*]\s+(.+)$", line)
        if bullet:
            if not list_stack or list_stack[-1] != "ul":
                close_lists(out, list_stack)
                out.append("<ul>")
                list_stack.append("ul")
            out.append(f"<li>{inline_markdown(bullet.group(1))}</li>")
            continue

        ordered = re.match(r"^\s*\d+\.\s+(.+)$", line)
        if ordered:
            if not list_stack or list_stack[-1] != "ol":
                close_lists(out, list_stack)
                out.append("<ol>")
                list_stack.append("ol")
            out.append(f"<li>{inline_markdown(ordered.group(1))}</li>")
            continue

        close_lists(out, list_stack)
        out.append(f"<p>{inline_markdown(line.strip())}</p>")

    if in_code:
        out.append("<pre><code>")
        out.append(html.escape("\n".join(code_lines)))
        out.append("</code></pre>")
    close_lists(out, list_stack)
    return "\n".join(out), headings


def layout(title: str, body: str, root_prefix: str = "") -> str:
    return f"""<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>{html.escape(title)} - Avena Docs</title>
    <link rel="stylesheet" href="{root_prefix}assets/docs.css" />
  </head>
  <body>
    <header class="topbar">
      <a class="brand" href="{root_prefix}index.html">Avena Docs</a>
      <nav>
        <a href="{root_prefix}docs/setup-guide/">Setup</a>
        <a href="{root_prefix}docs/schematics/">Schematics</a>
        <a href="{root_prefix}api/rust/">Rust API</a>
        <a href="{root_prefix}api/frontend/">Frontend API</a>
      </nav>
    </header>
    {body}
  </body>
</html>
"""


def write_doc(root: Path, site: Path, title: str, src: str, dest: str) -> None:
    source = root / src
    body, headings = render_markdown(source.read_text())
    toc = "\n".join(
        f'<li class="toc-level-{level}"><a href="#{slug}">{html.escape(text)}</a></li>'
        for level, text, slug in headings
        if level <= 3
    )
    page = f"""
    <main class="doc-shell">
      <aside class="toc">
        <a href="../../index.html">Docs home</a>
        <h2>On this page</h2>
        <ol>{toc}</ol>
      </aside>
      <article class="doc-content">
        {body}
      </article>
    </main>
"""
    output = site / dest
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(layout(title, page, "../../"))


def write_index(site: Path) -> None:
    cards = "\n".join(
        f"""
        <a class="card" href="{dest.removesuffix('index.html')}">
          <span>Guide</span>
          <strong>{html.escape(title)}</strong>
          <small>{html.escape(src)}</small>
        </a>
        """
        for title, src, dest in DOCS
    )
    body = f"""
    <main class="home">
      <section class="hero">
        <p class="eyebrow">Avena LabJack Pipeline</p>
        <h1>Setup, operations, and API reference for the edge data system.</h1>
        <p>
          This site is generated locally from the repository Markdown, Rust
          sources, and frontend TypeScript modules.
        </p>
        <div class="actions">
          <a class="button primary" href="docs/setup-guide/">Setup guide</a>
          <a class="button" href="docs/schematics/">Schematics</a>
          <a class="button" href="api/rust/">Rust API</a>
          <a class="button" href="api/frontend/">Frontend API</a>
        </div>
      </section>
      <section class="grid">
        {cards}
        <a class="card" href="api/rust/">
          <span>API</span>
          <strong>Rust API</strong>
          <small>Generated with cargo doc</small>
        </a>
        <a class="card" href="api/frontend/">
          <span>API</span>
          <strong>Frontend API</strong>
          <small>Generated with TypeDoc</small>
        </a>
      </section>
    </main>
"""
    (site / "index.html").write_text(layout("Home", body, ""))


def write_rust_api_index(site: Path) -> None:
    rust_api = site / "api" / "rust"
    crates = sorted(
        path.name
        for path in rust_api.iterdir()
        if path.is_dir() and (path / "index.html").exists() and not path.name.startswith(".")
    )
    cards = "\n".join(
        f"""
        <a class="card" href="{crate}/">
          <span>Rust binary</span>
          <strong>{html.escape(crate)}</strong>
          <small>Generated with cargo doc</small>
        </a>
        """
        for crate in crates
    )
    body = f"""
    <main class="home">
      <section class="hero">
        <p class="eyebrow">Rust API</p>
        <h1>Generated docs for the Rust LabJack pipeline binaries.</h1>
        <p>
          These pages are produced by <code>cargo doc --no-deps --document-private-items</code>.
        </p>
      </section>
      <section class="grid">{cards}</section>
    </main>
"""
    (rust_api / "index.html").write_text(layout("Rust API", body, "../../"))


def copy_assets(root: Path, site: Path) -> None:
    assets = site / "assets"
    assets.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(root / "docs-site" / "assets" / "docs.css", assets / "docs.css")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--site", required=True, type=Path)
    args = parser.parse_args()

    copy_assets(args.root, args.site)
    write_index(args.site)
    write_rust_api_index(args.site)
    for title, src, dest in DOCS:
        write_doc(args.root, args.site, title, src, dest)


if __name__ == "__main__":
    main()
