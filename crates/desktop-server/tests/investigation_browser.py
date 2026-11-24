"""Use real paper ingestion and a deterministic provider result to exercise the map."""

import hashlib
from pathlib import Path
import tempfile

from playwright.sync_api import expect
from recovery_example_browser import open_install


def check_investigation(browser, binary):
    lines = ["Assume a unit mass.", "The net force equals its acceleration.", "We measure a unit force.",
             r"The calibrated acceleration $a = F/m$ stays constant throughout the interval in which the same force acts on the mass.",
             "Calibration transfers the unit convention to the sensor."]
    body = "\n".join(lines)
    with tempfile.TemporaryDirectory(prefix="re-search-paper-map-") as directory:
        root = Path(directory)
        (root / "paper").mkdir()
        with open_install(browser, binary, root) as page:
            errors = []
            page.on("pageerror", lambda error: errors.append(str(error)))
            page.route("**/api/models*", lambda r: r.fulfill(json={"providers": [
                {"id": "antigravity", "label": "Antigravity", "route": "antigravity",
                 "models": [{"id": "gemini-carbon-fixture", "label": "Gemini fixture", "default": True}]}]}))
            page.evaluate("async () => (await import('/assets/model_picker.js')).loadModels(true)")
            page.get_by_role("button", name="Investigate a paper", exact=True).click()
            expect(page.locator("#investigation-setup")).to_be_visible()
            parent = page.evaluate("async () => (await (await import('/assets/api.js')).listSessions()).sessions[0].id")
            child = page.evaluate("async () => (await (await import('/assets/api.js')).createSession({title:'Linked research'})).id")
            state = {"map": None, "map_seq": None, "corrections": [], "activities": []}
            receipts = {}
            origin = {}
            scenario = {"running": False}

            def session_view(route):
                response = route.fetch()
                data = response.json()
                data["investigation"] = dict(state)
                data["running"] = scenario["running"]
                route.fulfill(response=response, json=data)

            def child_view(route):
                response = route.fetch()
                data = response.json()
                data["investigation"] = {"origin": origin}
                route.fulfill(response=response, json=data)

            page.route("**/api/sessions/" + parent, session_view)
            page.route("**/api/sessions/" + child, child_view)

            def send(route):
                request = route.request.post_data_json
                receipts["send"] = request
                source = request["investigation_source_id"]
                path = receipts["upload"]["primary_path"]
                def anchor(index):
                    return {"source_id": source, "path": path, "file_sha256": hashlib.sha256(body.encode()).hexdigest(),
                            "source_seq": receipts["upload"]["event_seq"], "start_line": index + 1, "end_line": index + 1,
                            "kind": "passage", "excerpt": lines[index]}
                nodes = [{"id": name, "sentence": lines[i], "main": i in (1, 3), "kind": kind, "anchors": [anchor(i)],
                          "questions": ["Which assumption connects the result?"] if i == 0 else []}
                         for i, (name, kind) in enumerate([("mass", "assumption"), ("force", "result"), ("lab", "measurement"),
                                                          ("second", "prediction"), ("calibrate", "method")])]
                state.update(source_id=source, question=request["prompt"], map_seq=11,
                             map={"title": "Dynamics of a unit mass", "coverage": "partial", "answer_node_ids": ["force"],
                                  "limitations": ["Illustrative text."], "nodes": nodes, "edges": [
                                      {"from": "mass", "to": "force", "kind": "depends_on", "basis": "inferred",
                                       "reason": "Unit mass connects force and acceleration.", "anchor": anchor(0)},
                                      {"from": "lab", "to": "force", "kind": "supports", "basis": "explicit",
                                       "reason": "The authors report a unit force.", "anchor": anchor(2)},
                                      {"from": "mass", "to": "calibrate", "kind": "depends_on", "basis": "explicit",
                                       "reason": "Calibration uses this unit convention.", "anchor": anchor(4)},
                                      {"from": "calibrate", "to": "second", "kind": "supports", "basis": "inferred",
                                       "reason": "The calibration supports the predicted acceleration.", "anchor": anchor(3)}]})
                scenario.update(running=True, completed=state["map"], summary={"source_id":source, "request_seq":9,
                    "what":{"text":"The paper studies the acceleration of a unit mass.", "anchors":[anchor(1)]},
                    "why":{"unstated":True},
                    "how":{"text":"The authors measure a unit force.", "anchors":[anchor(2)]}})
                state.update(map=None, map_seq=None)
                route.fulfill(status=202, json={"status": "running"})

            def upload(route):
                response = route.fetch()
                receipts["upload"] = response.json()["ingested"][0]
                route.fulfill(response=response)

            def correct(route):
                change = route.request.post_data_json
                state["corrections"].append(change)
                route.fulfill(status=201, json=change)

            def check(route):
                request = route.request.post_data_json
                state["activities"].append({**request, "status": "complete", "result": "The unit convention must be specified."})
                route.fulfill(status=202, json={"status": "running"})

            def linked(route):
                request = route.request.post_data_json
                origin.update(parent_session=parent, question=request["question"], claim=lines[1], node_id="force", map_seq=11,
                              assumptions=[lines[0]], sources=state["map"]["nodes"][0]["anchors"])
                route.fulfill(status=201, json={"session_id": child})

            page.route("**/" + parent + "/upload", upload)
            page.route("**/" + parent + "/messages", send)
            page.route("**/" + parent + "/investigation/corrections", correct)
            page.route("**/" + parent + "/investigation/rechecks", check)
            page.route("**/" + parent + "/investigation/research", linked)
            paper_file = page.get_by_label("Primary paper", exact=True)
            paper_file.set_input_files({"name": "range.pdf", "mimeType": "application/pdf", "buffer": b"%PDF-"})
            expect(page.get_by_label("First page")).to_be_visible()
            page.get_by_label("First page").fill("2")
            page.get_by_role("button", name="Build paper map").click()
            expect(page.locator("#investigation-status")).to_contain_text("Enter both page numbers")
            page.get_by_label("Primary paper", exact=True).set_input_files({"name": "dynamics.txt", "mimeType": "text/plain", "buffer": body.encode()})
            expect(page.get_by_label("First page")).to_be_hidden()
            page.get_by_label("Reading question").fill("Which assumption connects the result?")
            page.get_by_role("button", name="Build paper map").click()
            brief = page.locator("#investigation-summary")
            expect(brief.get_by_role("heading", name="What, Why, and How")).to_be_visible()
            expect(brief).to_contain_text("Reading the paper")
            state["summary"] = scenario["summary"]
            expect(brief).to_contain_text("The paper studies the acceleration of a unit mass.", timeout=15000)
            expect(brief).to_contain_text("The reviewed passages do not state this explicitly.")
            expect(page.locator("#investigation-map")).to_be_empty()
            brief.locator(".investigation-summary-part").first.locator("summary").first.click()
            scenario["running"] = False
            state.update(map=scenario["completed"], map_seq=11)
            page.evaluate("async id => (await import('/assets/investigation.js')).refreshInvestigation(id)", parent)
            expect(page.get_by_role("heading", name="Dynamics of a unit mass")).to_be_visible()
            expect(brief.locator("details").first).to_have_attribute("open", "")
            assert receipts["send"]["provider"] == "antigravity"
            assert "investigation_page_start" not in receipts["send"]
            assert receipts["send"]["model"] == "gemini-carbon-fixture"
            assert receipts["send"]["prompt"] == "Which assumption connects the result?"
            graph = page.locator("#investigation-map")
            expect(graph.locator(".investigation-graph-node")).to_have_count(3)
            expect(graph.locator(".question-match")).to_contain_text("Selected for your question")
            expect(graph.locator(".question-match")).to_contain_text(lines[1])
            expect(graph.locator('[data-node="second"] .katex').first).to_be_visible()
            edge = graph.locator('[data-from="mass"][data-to="second"]')
            expect(edge).to_have_attribute("data-hidden", "1")
            edge.focus()
            edge.press("Enter")
            graph.get_by_role("button", name="Expand connecting statements").click()
            expect(graph.locator('[data-node="calibrate"]')).to_be_visible()
            expect(graph.locator('[data-from="calibrate"][data-to="second"]')).to_have_count(1)
            graph.get_by_role("button", name="Collapse expanded branches").click()
            expect(graph.locator('[data-node="calibrate"]')).to_have_count(0)
            graph.get_by_role("button", name="Steps in order").click()
            expect(graph.locator(".investigation-index > .investigation-node").first).to_have_attribute("data-node", "mass")
            graph.get_by_role("button", name="Graph", exact=True).click()
            expect(graph.locator('[data-node="lab"]')).to_have_count(0)
            search = graph.get_by_role("searchbox")
            search.fill("measure")
            expect(graph.locator('[data-node="lab"]')).to_be_visible()
            search.fill("")
            page.set_viewport_size({"width": 390, "height": 844})
            if page.locator("#app-shell").get_attribute("data-rail-open") == "true":
                page.get_by_role("button", name="Close sessions").click()
            assert graph.locator(".investigation-graph-viewport").evaluate("el => el.clientWidth <= window.innerWidth && el.scrollWidth > el.clientWidth")
            graph.locator('[data-node="force"]').click()
            expect(graph.get_by_role("heading", name="What this statement uses")).to_be_visible()
            expect(graph.get_by_text("Dependency · Inferred rationale")).to_be_visible()
            expect(graph.get_by_text("Support · Author-stated connection")).to_be_visible()
            graph.locator(".investigation-selected summary").click()
            graph.get_by_role("button", name="Open cited lines").first.click()
            expect(page.locator("#source-reader-dialog")).to_be_visible()
            expect(page.locator("#source-reader-status")).to_contain_text("Line 2")
            page.keyboard.press("Escape")
            graph.get_by_role("button", name="Add correction").click()
            graph.get_by_label("Applies to").select_option("mass")
            graph.get_by_label("Your correction").fill("Specify the mass unit.")
            graph.get_by_role("button", name="Save correction").click()
            expect(graph.locator(".investigation-correction")).to_contain_text("Specify the mass unit.")
            expect(graph.locator(".investigation-claim-title")).to_have_text(lines[1])
            graph.get_by_role("button", name="Recheck claim").click()
            graph.get_by_label("Applies to").select_option("mass")
            graph.locator("form").get_by_role("button", name="Recheck claim").click()
            expect(graph.get_by_text("Recheck · complete")).to_be_visible()
            assert state["activities"][0]["map_seq"] == 11
            assert state["activities"][0]["edge_from"] == "mass"
            assert state["activities"][0]["provider"] == "antigravity"
            graph.get_by_role("button", name="Open linked Research").click()
            graph.get_by_label("Research question", exact=True).fill("Does this hold for another mass?")
            graph.locator("form").get_by_role("button", name="Open linked Research").click()
            banner = page.locator("#investigation-origin")
            expect(banner).to_be_visible()
            banner.locator("summary").first.click()
            expect(banner).to_contain_text(lines[0])
            banner.get_by_role("button", name="Use research question").click()
            expect(page.locator("#composer-input")).to_have_value("Does this hold for another mass?")
            banner.get_by_role("button", name="Open original claim").click()
            expect(graph.locator(".investigation-claim-title")).to_have_text(lines[1])
            expect(brief).to_contain_text("The authors measure a unit force.")
            assert not errors, errors
    print("Investigation upload, provider selection, focused map, exact passages, correction, recheck and linked Research passed")
