"""Exercise the local example and recover a manuscript after killing the server."""

from contextlib import contextmanager
import json
from pathlib import Path
import subprocess
import tempfile

from playwright.sync_api import expect


@contextmanager
def open_install(browser, binary, root):
    server = subprocess.Popen(
        [str(binary), "ui", "--data-dir", str(root / "data"), "--workspace", str(root / "paper"),
         "--port", "0", "--no-open", "true", "--print-url-json", "true", "--api-base", "http://127.0.0.1:9"],
        stdout=subprocess.PIPE, text=True,
    )
    context = browser.new_context(viewport={"width": 1280, "height": 820})
    try:
        page = context.new_page()
        page.goto(json.loads(server.stdout.readline())["url"])
        yield page
    finally:
        # End the server first. Recovery must work without a page-unload handler.
        server.kill()
        server.wait(timeout=10)
        server.stdout.close()
        context.close()


def open_manuscript(page):
    toggle = page.locator("#canvas-toggle")
    if toggle.get_attribute("aria-expanded") != "true":
        toggle.click()
    page.get_by_role("tab", name="LaTeX").click()
    return page.get_by_label("Manuscript editor")


def check_recovery_example(browser, binary):
    with tempfile.TemporaryDirectory(prefix="re-search-restart-") as temp:
        root = Path(temp)
        paper = root / "paper"
        paper.mkdir()
        manuscript = paper / "spring.tex"
        manuscript.write_text("First version", encoding="utf-8")
        with open_install(browser, binary, root) as page:
            steps = []
            page.on("request", lambda req: steps.append(req.url) if req.method == "POST" else None)
            page.get_by_role("button", name="Try a worked example offline").click()
            demo = page.get_by_role("dialog", name="Check a two-site equilibrium")
            demo.get_by_role("button", name="Both equations hold").click()
            demo.get_by_role("button", name="Run the calculation").click()
            expect(demo).to_contain_text("5/3, -4/3")
            demo.get_by_text("Inspect the calculation", exact=True).click()
            expect(demo.locator("pre")).to_contain_text("(2, 1) → (3, 0)")
            expect(demo.locator(".katex").first).to_be_visible()
            demo.get_by_role("button", name="Try again").click()
            demo.get_by_role("button", name="The proposal fails").click()
            demo.get_by_role("button", name="Run the calculation").click()
            expect(demo).to_contain_text("Your prediction was correct.")
            demo.get_by_role("button", name="Close worked example").click()
            assert not any("/api/sessions" in url for url in steps), steps

            editor = open_manuscript(page)
            page.get_by_role("button", name="Start a manuscript").click()
            expect(editor).to_have_value("First version")
            session = page.evaluate("async () => (await (await import('/assets/api.js')).listSessions()).sessions[0].id")
            other = page.context.new_page()
            token = page.evaluate("async () => (await import('/assets/api.js')).getCsrfToken()")
            other.set_extra_http_headers({"X-ReSearch-Token": token})
            other.goto(page.url)
            other_editor = open_manuscript(other)
            expect(other_editor).to_have_value("First version")
            attempts = []
            def first_write_fails(route):
                if route.request.method == "PUT":
                    attempts.append(1)
                    if len(attempts) == 1:
                        route.fulfill(status=503, json={"error": "Retry the snapshot"})
                        return
                route.continue_()
            page.route("**/manuscript-drafts", first_write_fails)
            editor.fill("My recovered derivation")
            expect(page.locator("#latex-recovery-status")).to_have_text("Recovery copy saved")
            assert len(attempts) >= 2
            other_editor.fill("An independent derivation")
            expect(other.locator("#latex-recovery-status")).to_have_text("Recovery copy saved")
            other.close()
            assert manuscript.read_text(encoding="utf-8") == "First version"
            # A second window that did not read the revision cannot wipe this draft.
            status = page.evaluate("""async session => {
              const api = await import('/assets/api.js');
              const [saved] = (await api.getManuscriptDrafts(session)).drafts;
              try { await api.putManuscriptDraft(session, {workspace:saved.workspace, path:saved.path, revision:null, draft:null}); }
              catch (e) { return e.status; }
            }""", session)
            assert status == 409

        manuscript.write_text("An external edit", encoding="utf-8")
        with open_install(browser, binary, root) as page:
            editor = open_manuscript(page)
            expect(editor).to_have_value("An external edit")
            expect(editor).to_have_attribute("readonly", "")
            expect(page.locator("#latex-recovery-list option")).to_have_count(2)
            page.get_by_role("button", name="Restore draft").click()
            expect(editor).to_have_value("My recovered derivation")
            page.locator("#latex-save-btn").click()
            expect(page.locator("#latex-reload-btn")).to_be_visible()
            assert manuscript.read_text(encoding="utf-8") == "An external edit"

        with open_install(browser, binary, root) as page:
            editor = open_manuscript(page)
            page.on("dialog", lambda dialog: dialog.accept())
            page.get_by_role("button", name="Discard draft").click()
            expect(page.locator("#latex-recovery-list option")).to_have_count(1)
            expect(page.locator("#latex-recovery-list option")).to_contain_text("alternate copy")
            page.get_by_role("button", name="Discard draft").click()
            expect(page.locator("#latex-recovery")).to_be_hidden()
            expect(editor).to_be_editable()
            expect(editor).to_have_value("An external edit")
            drafts = page.evaluate("async id => (await (await import('/assets/api.js')).getManuscriptDrafts(id)).drafts", session)
            assert drafts == []
        print("Offline computation, crash recovery, concurrent drafts, external edits and discard passed")
