"""Restore the project created in the workbench flow into an independent installation."""
import json
import subprocess

from playwright.sync_api import expect


def check_project_backup(browser, binary, root, original):
    archive = root / "project.research-backup.zip"
    with original.expect_download() as downloaded:
        original.get_by_role("button", name="Back up project").click()
    downloaded.value.save_as(archive)
    server = subprocess.Popen([
        str(binary), "ui", "--data-dir", str(root / "recovered-data"), "--port", "0",
        "--no-open", "true", "--print-url-json", "true", "--api-base", "http://127.0.0.1:9",
    ], stdout=subprocess.PIPE, text=True)
    context = browser.new_context(viewport={"width": 1440, "height": 900})
    errors = []
    try:
        page = context.new_page()
        page.on("pageerror", lambda error: errors.append(str(error)))
        page.goto(json.loads(server.stdout.readline())["url"])
        page.locator("#projects-open-btn").click()
        expect(page.locator("#project-select")).to_contain_text("No projects")
        with page.expect_file_chooser() as picker:
            page.get_by_role("button", name="Restore backup").click()
        picker.value.set_files(archive)
        expect(page.locator("#project-page-status")).to_contain_text("Restored Boundary terms")
        page.get_by_role("tab", name="Findings").click()
        expect(page.locator("#project-finding-list")).to_contain_text("Periodic boundary terms cancel.")
        page.locator(".project-session-link").first.click()
        if page.locator("#canvas-toggle").get_attribute("aria-expanded") != "true":
            page.locator("#canvas-toggle").click()
        page.get_by_role("tab", name="Sources").click()
        page.get_by_label("Search ingested paper passages").fill("periodic variations")
        page.locator("#paper-passage-form").get_by_role("button").click()
        expect(page.locator("#paper-passage-results")).to_contain_text("cancel the surface term")
        assert not errors, errors
        print("Project backup restored in a separate data directory with finding, session, and readable source.")
    finally:
        context.close()
        server.terminate()
        server.wait(timeout=10)

