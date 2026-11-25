"""Use real paper ingestion and a deterministic provider result to exercise the map."""

import hashlib
import os
from pathlib import Path
import tempfile

from playwright.sync_api import expect
from recovery_example_browser import open_install


def capture_graph_screen(page, name, target):
    folder = os.environ.get("RESEARCH_GRAPH_SCREENSHOTS")
    if not folder:
        return
    directory = Path(folder)
    directory.mkdir(parents=True, exist_ok=True)
    target.evaluate("el => el.scrollIntoView({block:'start', inline:'nearest'})")
    page.screenshot(path=directory / (name + ".png"), animations="disabled")


def check_investigation(browser, binary):
    lines = ["Assume a unit mass.", "The net force equals its acceleration.", "We measure a unit force.",
             r"The calibrated acceleration $a = F/m$ stays constant throughout the interval in which the same force acts on the mass.",
             r"Calibration transfers the unit convention $m=1$ to the sensor, so the measured force can be expressed in the same units across observations."]
    body = "\n".join(lines)
    with tempfile.TemporaryDirectory(prefix="re-search-paper-map-") as directory:
        root = Path(directory)
        (root / "paper").mkdir()
        def mock_provider(page):
            page.add_init_script("""const listen = EventSource.prototype.addEventListener;
                EventSource.prototype.addEventListener = function(name, ...args) {
                  if (name === 'turn_complete') window.fixtureEventStream = this;
                  return listen.call(this, name, ...args);
                };""")
            page.route("**/api/models*", lambda r: r.fulfill(json={"providers": [
                {"id": "antigravity", "label": "Antigravity", "route": "antigravity",
                 "models": [{"id": "gemini-carbon-fixture", "label": "Gemini fixture", "default": True}]}]}))
        with open_install(browser, binary, root, before_navigate=mock_provider) as page:
            errors = []
            page.on("pageerror", lambda error: errors.append(str(error)))
            expect(page.locator('#composer-model option:checked')).to_have_text('Gemini fixture')
            page.get_by_role("button", name="Investigate a paper", exact=True).click()
            expect(page.locator("#investigation-setup")).to_be_visible()
            expect(page.locator('#composer-model option:checked')).to_have_text('Gemini fixture')
            parent = page.evaluate("async () => (await (await import('/assets/api.js')).listSessions()).sessions[0].id")
            child = page.evaluate("async () => (await (await import('/assets/api.js')).createSession({title:'Linked research'})).id")
            state = {"map": None, "map_seq": None, "corrections": [], "activities": [], "branches": []}
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
                                      {"from": "calibrate", "to": "force", "kind": "supports", "basis": "inferred",
                                       "reason": "The sensor's calibration supports the inferred acceleration.", "anchor": anchor(4)},
                                      {"from": "calibrate", "to": "second", "kind": "supports", "basis": "inferred",
                                       "reason": "The calibration supports the predicted acceleration.", "anchor": anchor(3)}]})
                scenario.update(running=True, completed=state["map"], summary={"source_id":source, "request_seq":9,
                    "what":{"text":"The paper studies the acceleration of a unit mass.", "anchors":[anchor(1)]},
                    "why":{"unstated":True},
                    "how":{"text":"The authors measure a unit force.", "anchors":[anchor(2)]}})
                scenario["conventions"] = {"source_id":source, "coverage":"partial", "limitations":[], "done":False, "items":[
                    {"id":"unit-mass", "category":"notation", "label":"$m$", "meaning":"Mass equals one in the chosen units.",
                     "scope":"The force calculation", "basis":"explicit", "anchors":[anchor(0)]},
                    {"id":"calibration", "category":"normalization", "label":"Calibration units", "meaning":"The sensor uses the same mass unit.",
                     "scope":"Sensor observations", "basis":"inferred", "rationale":"The calibration sentence links the unit convention to the sensor.",
                     "anchors":[anchor(4)]}]}
                state.update(map=None, map_seq=None, mapping_requested=True, conventions_requested=True)
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

            def develop(route):
                request = route.request.post_data_json
                state["branches"].append({**request, "request_seq": len(state["branches"]) + 40, "status": "running"})
                scenario["running"] = True
                route.fulfill(status=202, json={"status": "running"})

            def conventions(route):
                receipts["conventions"] = route.request.post_data_json
                scenario["running"] = True
                state["conventions_requested"] = True
                route.fulfill(status=202, json={"status": "running"})

            def linked(route):
                request = route.request.post_data_json
                origin.update(parent_session=parent, question=request["question"], claim=lines[1], node_id="force", map_seq=11,
                              assumptions=[lines[0]], sources=state["map"]["nodes"][0]["anchors"])
                route.fulfill(status=201, json={"session_id": child})

            receipts["stops"] = []
            def stop_turn(route):
                receipts["stops"].append(True)
                route.fulfill(status=200, json={"ok": True})

            page.route("**/" + parent + "/upload", upload)
            page.route("**/" + parent + "/messages", send)
            page.route("**/" + parent + "/stop", stop_turn)
            page.route("**/" + parent + "/investigation/corrections", correct)
            page.route("**/" + parent + "/investigation/rechecks", check)
            page.route("**/" + parent + "/investigation/branches", develop)
            page.route("**/" + parent + "/investigation/conventions", conventions)
            page.route("**/" + parent + "/investigation/research", linked)
            paper_file = page.get_by_label("Primary paper", exact=True)
            paper_file.set_input_files({"name": "range.pdf", "mimeType": "application/pdf", "buffer": b"%PDF-"})
            expect(page.get_by_label("First page")).to_be_visible()
            page.get_by_label("First page").fill("2")
            page.get_by_role("button", name="Find main results").click()
            expect(page.locator("#investigation-status")).to_contain_text("Enter both page numbers")
            page.get_by_label("Primary paper", exact=True).set_input_files({"name": "dynamics.txt", "mimeType": "text/plain", "buffer": body.encode()})
            expect(page.get_by_label("First page")).to_be_hidden()
            page.get_by_role("button", name="Find main results").click()
            brief = page.locator("#investigation-summary")
            expect(brief.get_by_role("heading", name="What, Why, and How")).to_be_visible()
            expect(page.locator("#pipeline-bar")).to_be_hidden()
            expect(page.get_by_role("button", name="Paper", exact=True)).to_be_visible()
            expect(page.get_by_role("button", name="Conversation", exact=True)).to_be_visible()
            expect(brief).to_contain_text("Reading the paper")
            state["summary"] = scenario["summary"]
            state["conventions"] = scenario["conventions"]
            expect(brief).to_contain_text("The paper studies the acceleration of a unit mass.", timeout=15000)
            expect(brief).to_contain_text("The reviewed passages do not state this explicitly.")
            expect(page.locator("#investigation-map")).to_be_empty()
            catalog = page.locator("#investigation-conventions")
            expect(catalog.locator(":scope > summary")).to_contain_text("2 found so far")
            catalog.locator(":scope > summary").click()
            expect(catalog.get_by_role("heading", name="Notation")).to_be_visible()
            expect(catalog.locator('[data-entry="unit-mass"] .katex')).to_be_visible()
            inferred = catalog.locator('[data-entry="calibration"]')
            inferred.locator(":scope > summary").click()
            expect(inferred).to_contain_text("Inferred from use")
            expect(inferred).to_contain_text("The calibration sentence")
            expect(inferred).to_contain_text("Scope · Sensor observations")
            inferred.locator("[data-anchor] > summary").click()
            capture_graph_screen(page, "investigation-conventions-desktop", catalog)
            inferred.get_by_role("button", name="Open cited lines").click()
            expect(page.locator("#source-reader-status")).to_contain_text("Line 5")
            page.keyboard.press("Escape")
            catalog.locator(":scope > summary").click()
            brief.locator(".investigation-summary-part").first.locator("summary").first.click()
            scenario["running"] = False
            completed = scenario["completed"]
            state.update(map={**completed, "edges":[], "nodes":[n for n in completed["nodes"] if n["main"]]}, map_seq=11,
                         conventions_error="The provider stopped before the next batch.")
            page.evaluate("() => window.fixtureEventStream.dispatchEvent(new MessageEvent('turn_complete',{data:'{}'}))")
            expect(page.get_by_role("heading", name="Dynamics of a unit mass")).to_be_visible()
            expect(page.locator("#stream-status-pill")).to_have_text("Ready")
            expect(brief.locator("details").first).to_have_attribute("open", "")
            brief.locator(".investigation-summary-part").first.locator("summary").first.click()
            capture_graph_screen(page, "investigation-summary-desktop", brief)
            catalog.locator(":scope > summary").click()
            expect(catalog).to_contain_text("Conventions review stopped. The provider stopped before the next batch.")
            catalog.get_by_role("button", name="Continue conventions review").click()
            expect(page.locator("#composer-stop")).to_be_visible()
            expect(catalog.locator(":scope > summary")).to_contain_text("2 found so far")
            expect(catalog.locator(":scope > summary")).to_be_focused()
            assert receipts["conventions"]["map_seq"] == 11 and receipts["conventions"]["provider"] == "antigravity"
            for index in range(5):
                state["conventions"]["items"].append({**scenario["conventions"]["items"][0], "id":"note-"+str(index),
                    "label":"Reading "+str(index), "meaning":"A separate fixture convention."})
            page.evaluate("async id => (await import('/assets/investigation.js')).refreshInvestigation(id)", parent)
            expect(catalog.get_by_role("searchbox")).to_be_visible()
            catalog.get_by_role("searchbox").focus()
            page.keyboard.press("Escape")
            assert receipts["stops"] == []
            expect(page.locator("#composer-stop")).to_be_visible()
            inferred.get_by_role("button", name="Open cited lines").focus()
            state["conventions"].update(done=True, coverage="partial", limitations=["The appendix was outside the selected pages."])
            scenario["running"] = False
            page.evaluate("() => window.fixtureEventStream.dispatchEvent(new MessageEvent('turn_complete',{data:'{}'}))")
            expect(catalog.locator(":scope > summary")).to_contain_text("7 entries · partial coverage", timeout=15000)
            expect(inferred.get_by_role("button", name="Open cited lines")).to_be_focused()
            expect(catalog).to_contain_text("The appendix was outside the selected pages.")
            catalog.get_by_role("searchbox").fill("sensor observations")
            expect(catalog.locator("[data-entry]")).to_have_count(1)
            catalog.get_by_role("searchbox").fill("")
            catalog.locator(":scope > summary").click()
            assert receipts["send"]["provider"] == "antigravity"
            assert "investigation_page_start" not in receipts["send"]
            assert receipts["send"]["model"] == "gemini-carbon-fixture"
            assert receipts["send"]["prompt"] == "Identify the paper’s main results."
            assert receipts["send"]["investigation_scope"] == "map"
            graph = page.locator("#investigation-map")
            expect(graph.locator(".investigation-graph-node")).to_have_count(0)
            expect(graph.locator('.investigation-index [data-node]')).to_have_count(2)
            expect(graph.locator('[data-node="second"] .katex').first).to_be_visible()
            graph.locator('[data-node="force"]').click()
            expect(graph.get_by_label("Focus question")).to_have_value("What assumptions, methods and evidence support this result?")
            assert state["activities"] == [] and state["branches"] == []
            graph.get_by_label("Focus question").fill("How does the calibration support the argument?")
            graph.get_by_role("button", name="Develop this result").click()
            expect(graph.get_by_text("Developing branch · How does the calibration support the argument?")).to_be_visible()
            assert state["branches"][0]["provider"] == "antigravity" and state["branches"][0]["node_id"] == "force"
            state["branches"][0]["status"] = "built"
            scenario["running"] = False
            state["map"] = {**completed, "edges":[e for e in completed["edges"] if e["to"] != "second"]}
            page.evaluate("async id => (await import('/assets/investigation.js')).refreshInvestigation(id)", parent)
            chart = graph.locator(".investigation-index")
            expect(chart.locator(".investigation-graph-node")).to_have_count(2)
            edge = chart.locator('[data-from="mass"][data-to="force"]')
            expect(edge).to_have_attribute("data-hidden", "1")
            edge.focus()
            edge.press("Enter")
            graph.get_by_role("button", name="Expand connecting statements").click()
            expect(chart.locator('.investigation-graph-node [data-node="calibrate"]')).to_be_visible()
            expect(chart.locator('[data-node="calibrate"] .katex').first).to_be_visible()
            expect(chart.locator('[data-from="calibrate"][data-to="force"]')).to_have_count(1)
            capture_graph_screen(page, "investigation-graph-desktop", chart)
            graph.get_by_role("button", name="Collapse expanded branches").click()
            expect(chart.locator('[data-node="calibrate"]')).to_have_count(0)
            graph.get_by_role("button", name="Steps in order").click()
            expect(graph.locator(".investigation-index > .investigation-node").first).to_have_attribute("data-node", "mass")
            graph.get_by_role("button", name="Graph", exact=True).click()
            expect(chart.locator('[data-node="lab"]')).to_have_count(0)
            search = graph.get_by_role("searchbox")
            search.fill("measure")
            expect(chart.locator('[data-node="lab"]')).to_be_visible()
            search.fill("")
            page.set_viewport_size({"width": 390, "height": 844})
            # The resize listener closes the desktop rail on entry to a narrow viewport.
            expect(page.locator("#rail-toggle")).to_have_attribute("aria-expanded", "false")
            expect(page.locator("#left-rail")).to_have_attribute("inert", "")
            graph.get_by_role("button", name="Show all statements").click()
            assert graph.locator(".investigation-graph-viewport").evaluate("el => el.clientWidth <= window.innerWidth && el.scrollWidth > el.clientWidth")
            capture_graph_screen(page, "investigation-graph-narrow", chart)
            graph.get_by_role("button", name="← Main results").click()
            capture_graph_screen(page, "investigation-summary-narrow", brief)
            graph.locator('[data-node="second"]').click()
            expect(graph.locator(".investigation-graph-viewport")).to_have_count(0)
            expect(graph.get_by_label("Focus question")).to_have_value("What assumptions, methods and evidence support this result?")
            graph.get_by_label("Focus question").fill("How was the acceleration prediction calibrated?")
            graph.get_by_role("button", name="Develop this result").click()
            expect(graph.get_by_text("Developing branch · How was the acceleration prediction calibrated?")).to_be_visible()
            state["branches"][1]["status"] = "built"
            assert state["branches"][1]["node_id"] == "second"
            scenario["running"] = False
            state["map"] = completed
            page.evaluate("async id => (await import('/assets/investigation.js')).refreshInvestigation(id)", parent)
            expect(graph.locator(".investigation-graph-viewport")).to_be_visible()
            expect(graph.locator('.investigation-graph-viewport [data-node="force"]')).to_have_count(0)
            graph.get_by_role("button", name="← Main results").click()
            graph.locator('[data-node="force"]').click()
            expect(graph.locator(".investigation-graph-viewport")).to_be_visible()
            assert len(state["branches"]) == 2 and state["activities"] == []
            expect(graph.get_by_role("heading", name="What this statement uses")).to_be_visible()
            expect(graph.get_by_text("Dependency · Inferred rationale")).to_be_visible()
            expect(graph.get_by_text("Support · Author-stated connection")).to_be_visible()
            expect(graph.locator(".investigation-selected details").first).to_have_attribute("open", "")
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
            expect(catalog.locator(":scope > summary")).to_contain_text("7 entries · partial coverage")
            # Earlier versions stored result maps before catalogs existed.
            state.update(conventions=None, conventions_requested=False)
            page.evaluate("async id => (await import('/assets/investigation.js')).refreshInvestigation(id)", parent)
            catalog.locator(":scope > summary").click()
            catalog.get_by_role("button", name="Find paper conventions").click()
            expect(catalog.locator(":scope > summary")).to_contain_text("Reading the paper")
            state.update(conventions={"done":True,"coverage":"paper","items":[],"limitations":["The paper did not establish an explicit convention."]})
            scenario["running"] = False
            page.evaluate("async id => (await import('/assets/investigation.js')).refreshInvestigation(id)", parent)
            expect(catalog.locator(":scope > summary")).to_contain_text("None reported in the paper")
            expect(graph.locator(".investigation-claim-title")).to_have_text(lines[1])
            assert not errors, errors
    check_question_first_investigation(browser, binary)
    print("Investigation upload, provider selection, focused map, exact passages, correction, recheck and linked Research passed")


def check_question_first_investigation(browser, binary):
    lines = ["Assume a unit mass.", "The net force equals its acceleration.", "We measure a unit force."]
    body = "\n".join(lines)
    with tempfile.TemporaryDirectory(prefix="re-search-paper-question-") as directory:
        root = Path(directory)
        no_models_root = root / "no-models"
        (no_models_root / "paper").mkdir(parents=True)
        question_root = root / "question"
        (question_root / "paper").mkdir(parents=True)

        def mock_no_models(page):
            page.route("**/api/models*", lambda r: r.fulfill(json={"providers": []}))

        with open_install(browser, binary, no_models_root, before_navigate=mock_no_models) as page:
            errors = []
            page.on("pageerror", lambda error: errors.append(str(error)))
            uploads = []
            page.route("**/upload", lambda r: (uploads.append(True), r.continue_()))
            page.get_by_role("button", name="Investigate a paper", exact=True).click()
            expect(page.locator("#investigation-setup")).to_be_visible()
            page.get_by_label("Primary paper", exact=True).set_input_files(
                {"name": "dynamics.txt", "mimeType": "text/plain", "buffer": body.encode()}
            )
            page.get_by_role("button", name="Find main results").click()
            expect(page.locator("#settings-dialog")).to_be_visible()
            assert uploads == [], "upload must not run when no model is connected"
            assert not errors, errors

        def mock_provider(page):
            page.add_init_script("""const listen = EventSource.prototype.addEventListener;
                EventSource.prototype.addEventListener = function(name, ...args) {
                  if (name === 'turn_complete') window.fixtureEventStream = this;
                  return listen.call(this, name, ...args);
                };""")
            page.route("**/api/models*", lambda r: r.fulfill(json={"providers": [
                {"id": "antigravity", "label": "Antigravity", "route": "antigravity",
                 "models": [{"id": "gemini-carbon-fixture", "label": "Gemini fixture", "default": True}]}]}))

        with open_install(browser, binary, question_root, before_navigate=mock_provider) as page:
            errors = []
            page.on("pageerror", lambda error: errors.append(str(error)))
            expect(page.locator('#composer-model option:checked')).to_have_text('Gemini fixture')
            page.get_by_role("button", name="Investigate a paper", exact=True).click()
            expect(page.locator("#investigation-setup")).to_be_visible()
            expect(page.locator('#composer-model option:checked')).to_have_text('Gemini fixture')
            sid = page.evaluate("async () => (await (await import('/assets/api.js')).listSessions()).sessions[0].id")
            state = {"map": None, "map_seq": None, "mapping_requested": False, "answers": [],
                     "corrections": [], "activities": [], "branches": []}
            scenario = {"running": False, "upload": None}
            sends = []

            def session_view(route):
                response = route.fetch()
                data = response.json()
                data["investigation"] = dict(state)
                data["running"] = scenario["running"]
                route.fulfill(response=response, json=data)

            def upload(route):
                response = route.fetch()
                scenario["upload"] = response.json()["ingested"][0]
                route.fulfill(response=response)

            def send(route):
                req = route.request.post_data_json
                sends.append(req)
                source = req.get("investigation_source_id") or state.get("source_id")
                path = scenario["upload"]["primary_path"]
                state["source_id"] = source
                if req.get("investigation_scope") == "map":
                    anchor = {"source_id": source, "path": path,
                              "file_sha256": hashlib.sha256(body.encode()).hexdigest(),
                              "source_seq": scenario["upload"]["event_seq"],
                              "start_line": 2, "end_line": 2, "kind": "passage", "excerpt": lines[1]}
                    state.update(
                        question=req["prompt"], mapping_requested=True, map_seq=11,
                        map={"title": "Dynamics of a unit mass", "coverage": "partial",
                             "answer_node_ids": ["force"], "limitations": [],
                             "nodes": [{"id": "force", "sentence": lines[1], "main": True,
                                        "kind": "result", "anchors": [anchor], "questions": []}],
                             "edges": []},
                    )
                elif len(state["answers"]) == 0:
                    state["answers"] = [{
                        "seq": 3, "question": req["prompt"], "status": "answered",
                        "text": "Unit mass [L1] connects force [L2].",
                        "cites": [{"start": 1, "end": 1, "read": True}, {"start": 2, "end": 2, "read": False}],
                    }]
                else:
                    state["answers"] = [*state["answers"], {
                        "seq": 20, "question": req["prompt"], "status": "answered",
                        "text": "The measured force uses unit mass [L3].",
                        "cites": [{"start": 3, "end": 3, "read": True}],
                    }]
                scenario["running"] = True
                route.fulfill(status=202, json={"status": "running"})

            page.route("**/api/sessions/" + sid, session_view)
            page.route("**/" + sid + "/upload", upload)
            page.route("**/" + sid + "/messages", send)

            question = "Which assumption connects the result?"
            page.get_by_label("Your question").fill(question)
            expect(page.locator("#investigation-setup button[type='submit']")).to_have_text("Answer my question")
            page.get_by_label("Primary paper", exact=True).set_input_files(
                {"name": "dynamics.txt", "mimeType": "text/plain", "buffer": body.encode()}
            )
            with page.expect_response("**/" + sid + "/messages"):
                page.get_by_role("button", name="Answer my question").click()
            assert sends[0]["investigation_scope"] == "question"
            assert sends[0]["prompt"] == question
            scenario["running"] = False
            page.evaluate("() => window.fixtureEventStream.dispatchEvent(new MessageEvent('turn_complete',{data:'{}'}))")

            answer = page.locator("#investigation-answer")
            expect(answer).to_be_visible()
            expect(answer).to_contain_text("Unit mass [L1] connects force [L2].")
            expect(page.locator("#investigation-setup")).to_be_hidden()
            expect(page.locator("#investigation-summary")).to_be_hidden()
            expect(page.locator("#investigation-conventions")).to_be_hidden()
            expect(page.locator("#composer-input")).to_be_visible()
            expect(page.locator("#composer-input")).to_have_attribute("placeholder", "Ask a follow-up about this paper…")

            cite = answer.locator(".line-cite")
            expect(cite).to_have_count(1)
            expect(cite).to_have_text("L1")
            cite.click()
            expect(page.locator("#source-reader-dialog")).to_be_visible()
            expect(page.locator("#source-reader-status")).to_contain_text("Line 1")
            page.keyboard.press("Escape")

            with page.expect_response("**/" + sid + "/messages"):
                answer.get_by_role("button", name="Find main results").click()
            assert sends[1]["investigation_scope"] == "map"
            assert sends[1]["prompt"] == question
            scenario["running"] = False
            page.evaluate("() => window.fixtureEventStream.dispatchEvent(new MessageEvent('turn_complete',{data:'{}'}))")
            expect(page.get_by_role("heading", name="Dynamics of a unit mass")).to_be_visible()
            expect(page.locator("#investigation-map .investigation-index")).to_contain_text("Selected for your question")
            page.locator('#investigation-map [data-node="force"]').click()
            expect(page.locator("#investigation-map").get_by_label("Focus question")).to_have_value(question)

            page.locator("#composer-input").fill("How is the force measured?")
            with page.expect_response("**/" + sid + "/messages"):
                page.locator("#composer-send").click()
            assert sends[2]["prompt"] == "How is the force measured?"
            scenario["running"] = False
            page.evaluate("() => window.fixtureEventStream.dispatchEvent(new MessageEvent('turn_complete',{data:'{}'}))")
            expect(answer).to_contain_text("The measured force uses unit mass [L3].")
            expect(answer.locator("details > summary")).to_have_text("Earlier questions (1)")
            assert not errors, errors

