"""File existing sessions, restart, and remove a folder without removing its sessions."""

from pathlib import Path
import tempfile
from playwright.sync_api import expect
from recovery_example_browser import open_install


def check_session_folders(browser, binary):
    with tempfile.TemporaryDirectory(prefix="re-search-folders-") as temp:
        root = Path(temp)
        (root / "paper").mkdir()
        with open_install(browser, binary, root) as page:
            expect(page.get_by_role("heading", name="Start a research question")).to_be_visible()
            page.evaluate("""async () => {
              const api = await import('/assets/api.js');
              for (const title of ['Spring modes', 'Mirror modes']) await api.createSession({title});
            }""")
            page.reload()
            expect(page.locator(".session-row")).to_have_count(2)
            page.get_by_role("button", name="Folders…").click()
            page.get_by_label("Folder name", exact=True).fill("Lattice")
            page.get_by_role("button", name="Create folder").click()
            expect(page.locator(".session-row")).to_have_count(0)
            filters = page.get_by_label("Filter sessions by folder")
            filters.select_option(label="All folders")
            page.locator(".session-row").filter(has_text="Spring modes").hover()
            page.get_by_role("button", name="Move Spring modes to folder").click()
            page.get_by_label("Destination folder").select_option(label="Lattice")
            page.get_by_role("button", name="Move session", exact=True).click()
            filters.select_option(label="Unfiled")
            expect(page.locator(".session-row")).to_have_count(1)
            expect(page.locator(".session-row")).to_contain_text("Mirror modes")
            page.get_by_role("searchbox", name="Filter sessions", exact=True).fill("Spring")
            expect(page.locator(".session-row")).to_have_count(0)
            filters.select_option(label="Lattice")
            expect(page.locator(".session-row")).to_have_count(1)
            page.get_by_role("button", name="Archived", exact=False).click()
            expect(page.locator(".session-row")).to_have_count(0)
            page.get_by_role("button", name="Folders…").click()
            page.get_by_label("Folder name", exact=True).fill("Spring systems")
            page.get_by_role("button", name="Rename folder").click()
            expect(filters.locator("option:checked")).to_have_text("Spring systems")
            stale = page.evaluate("""async () => {
              const api=await import('/assets/api.js');
              try { await api.changeSessionFolder({op:'create', name:'Stale', revision:0}); }
              catch (e) { return e.status; }
            }""")
            assert stale == 409

        with open_install(browser, binary, root) as page:
            filters = page.get_by_label("Filter sessions by folder")
            filters.select_option(label="Spring systems")
            expect(page.locator(".session-row")).to_have_count(1)
            expect(page.locator(".session-row")).to_contain_text("Spring modes")
            page.locator(".session-row").filter(has_text="Spring modes").hover()
            page.get_by_role("button", name="Move Spring modes to folder").click()
            page.get_by_label("Destination folder").select_option(label="Unfiled")
            page.get_by_role("button", name="Move session", exact=True).click()
            expect(page.locator(".session-row")).to_have_count(0)
            page.get_by_role("button", name="New research", exact=True).click()
            expect(page.locator(".session-row")).to_have_count(1)
            page.get_by_role("button", name="Folders…").click()
            page.once("dialog", lambda dialog: dialog.accept())
            page.get_by_role("button", name="Remove folder").click()
            filters.select_option(label="Unfiled")
            expect(page.locator(".session-row")).to_have_count(3)
        print("Folder creation, filing, search/archive filters, rename, restart, unfile and removal passed")
