"""Exercise the built workbench using Playwright and an isolated local data directory."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
from zipfile import ZipFile

from playwright.sync_api import expect, sync_playwright
from research_display_browser import check_research_display
from project_backup_browser import check_project_backup
from pdf_preview_browser import check_pdf_preview
from recovery_example_browser import check_recovery_example
from investigation_browser import check_investigation
from session_folders_browser import check_session_folders


def check_workbench(browser, binary, width, height):
    with tempfile.TemporaryDirectory(prefix="re-search-browser-") as temp:
        root = Path(temp)
        workspace = root / "manuscript"
        workspace.mkdir()
        (workspace / "field.tex").write_text(r"\section{Boundary conditions}", encoding="utf-8")
        (workspace / "notes.tex").write_text("Notes", encoding="utf-8")
        server = subprocess.Popen(
            [str(binary), "ui", "--data-dir", str(root / "data"), "--workspace", str(workspace),
             "--port", "0", "--no-open", "true", "--print-url-json", "true",
             "--api-base", "http://127.0.0.1:9"],
            stdout=subprocess.PIPE, text=True,
        )
        context = browser.new_context(viewport={"width": width, "height": height})
        errors = []
        page = context.new_page()
        if width == 1024:
            # Reproduce a save request arriving during a gap in the live event stream.
            page.add_init_script("""const listen = EventSource.prototype.addEventListener;
                EventSource.prototype.addEventListener = function(name, ...args) {
                    if (name !== 'approval_requested') return listen.call(this, name, ...args);
                };""")
        page.on("pageerror", lambda error: errors.append(str(error)))
        try:
            page.goto(json.loads(server.stdout.readline())["url"])
            expect(page.get_by_role("heading", name="Start a research question")).to_be_visible()
            expect(page.locator("#canvas-toggle")).to_have_attribute("aria-expanded", "false")
            page.get_by_role("button", name="Investigate a conjecture").click()
            expect(page.locator("#composer-input")).to_be_focused()
            assert "square complex matrices" in page.locator("#composer-input").input_value()
            options = page.locator("#research-options summary")
            options.focus()
            options.press("Enter")
            page.get_by_role("button", name="Interactive", exact=True).click()
            expect(page.locator('[data-rigor-preset="interactive"]')).to_have_attribute("aria-pressed", "true")
            page.keyboard.press("Escape")
            expect(options).to_be_focused()
            expect(page.locator("#research-options")).not_to_have_attribute("open", "")
            options.click()
            page.get_by_role("button", name="Conventions", exact=True).click()
            conventions = page.get_by_label("Pinned mathematical and physics conventions", exact=True)
            expect(conventions).to_be_focused()
            conventions.fill("Work in natural units.")
            page.get_by_role("button", name="Save Conventions").click()
            expect(options).to_be_focused()
            options.click()
            page.get_by_role("button", name="Conventions", exact=True).click()
            expect(conventions).to_have_value("Work in natural units.")
            conventions.press("Escape")
            expect(options).to_be_focused()
            page.locator("#composer-input").fill("")

            page.get_by_role("button", name="Open a research project").click()
            page.get_by_role("button", name="New project", exact=True).click()
            page.get_by_label("Project name").fill("Boundary terms")
            page.get_by_label("Research question or conjecture").fill("Which variations preserve the action?")
            page.get_by_role("button", name="Save project").click()
            expect(page.locator("#project-title")).to_have_text("Boundary terms")
            page.get_by_role("tab", name="Findings").click()
            page.get_by_label("Add a finding or failed approach").fill("Periodic boundary terms cancel.")
            page.get_by_role("button", name="Add note").click()
            expect(page.locator("#project-finding-list")).to_contain_text("Periodic boundary terms cancel.")
            page.get_by_role("button", name="New session", exact=True).click()
            expect(page.locator("#project-overview")).to_be_hidden()

            # Attachments can be ingested and inspected even without a model.
            page.locator("#composer-upload-input").set_input_files({
                "name": "variation.txt", "mimeType": "text/plain",
                "buffer": b"Boundary conditions\nPeriodic variations cancel the surface term.\n",
            })
            page.get_by_role("button", name="Send prompt", exact=True).click()
            expect(page.locator("#composer-files")).to_be_hidden()
            canvas_toggle = page.locator("#canvas-toggle")
            if canvas_toggle.get_attribute("aria-expanded") != "true":
                canvas_toggle.click()
            sources = page.get_by_role("tab", name="Sources")
            expect(sources).to_be_in_viewport()
            sources.click()
            page.get_by_label("Search ingested paper passages").fill("periodic variations")
            page.locator("#paper-passage-form").get_by_role("button").click()
            expect(page.locator("#paper-passage-results")).to_contain_text("cancel the surface term")
            page.get_by_role("tab", name="LaTeX").click()
            manuscript = page.get_by_label("Manuscript editor")
            files = page.locator("#latex-file-select")
            files.select_option("field.tex")
            expect(manuscript).to_have_value(r"\section{Boundary conditions}")
            manuscript.fill(r"\section{Revised boundary conditions}")
            files.select_option("notes.tex")
            expect(manuscript).to_have_value("Notes")
            files.select_option("field.tex")
            expect(manuscript).to_have_value(r"\section{Revised boundary conditions}")
            page.evaluate("async () => (await import('/assets/api.js')).updateProviderSettings({auto_approve_tools:false})")
            page.locator("#latex-save-btn").click()
            decision = page.locator(".approval-card").get_by_role("button", name=re.compile(r"^Approve (Once|& Run)$"))
            expect(decision).to_be_in_viewport(timeout=20000)
            decision.click()
            expect(page.locator("#stream-status-pill")).to_have_text("Ready")
            if canvas_toggle.get_attribute("aria-expanded") != "true":
                canvas_toggle.click()
            expect(page.locator("#latex-save-btn")).to_be_enabled()
            assert (workspace / "field.tex").read_text(encoding="utf-8") == r"\section{Revised boundary conditions}"
            with page.expect_download() as exported:
                page.get_by_role("button", name="Reproducible ZIP").click()
            with ZipFile(exported.value.path()) as bundle:
                assert {"report.tex", "references.bib", "report.html"}.issubset(bundle.namelist())
            if width <= 1120:
                page.get_by_role("button", name="Close Canvas").click()
                expect(canvas_toggle).to_be_focused()

            page.reload()
            page.locator("#projects-open-btn").click()
            expect(page.locator("#project-title")).to_have_text("Boundary terms")
            page.get_by_role("tab", name="Findings").click()
            expect(page.locator("#project-finding-list")).to_contain_text("Periodic boundary terms cancel.")
            if width == 1440:
                check_project_backup(browser, binary, root, page)
            assert not errors, errors
            print(f"{width}×{height}: draft, research options, project, ingestion, evidence, ZIP, and reload passed")
        finally:
            context.close()
            server.terminate()
            server.wait(timeout=10)


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    executable = "re-search.exe" if os.name == "nt" else "re-search"
    cli.add_argument("--binary", type=Path, default=Path("target/debug") / executable)
    cli.add_argument("--chromium", help="Use a system Chromium executable")
    args = cli.parse_args()
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(executable_path=args.chromium)
        try:
            for width, height in [(1440, 900), (1024, 768), (390, 844)]:
                check_workbench(browser, args.binary.resolve(), width, height)
            check_research_display(browser)
            check_pdf_preview(browser, args.binary.resolve())
            check_recovery_example(browser, args.binary.resolve())
            check_investigation(browser, args.binary.resolve())
            check_session_folders(browser, args.binary.resolve())
        finally:
            browser.close()


if __name__ == "__main__":
    main()
