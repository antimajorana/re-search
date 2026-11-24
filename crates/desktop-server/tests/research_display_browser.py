"""Browser regression cases for equation rendering, proof citations and live scrollback."""

from pathlib import Path
from urllib.parse import urlsplit

from playwright.sync_api import expect

root = Path(__file__).resolve().parents[1]
entry = {
    "entry_id": "e-1",
    "entry_type": "derivation",
    "summary": r"Simplify \[\frac{a}{b}\]",
    "latex": r"\(\Gamma=\frac{a}{b}\)",
    "details": r"Divide by $b \ne 0$.",
    "depends_on": [],
    "seq": 1,
}
html = """<!doctype html><meta charset="utf-8">
<link rel="stylesheet" href="/vendor/katex/katex.min.css">
<link rel="stylesheet" href="/assets/tokens.css">
<link rel="stylesheet" href="/assets/components.css">
<link rel="stylesheet" href="/assets/dag.css">
<script src="/vendor/katex/katex.min.js"></script>
<script src="/vendor/katex/auto-render.min.js"></script>
<div id="message-stream" style="height:240px;overflow-y:auto"></div>
<div id="dag-viewport" style="width:100%;height:240px">
<svg id="proof-dag-svg" width="100%" height="240"></svg></div>
<div id="dag-node-detail"></div>
<div id="used-sources-list"></div>
<details id="session-literature" hidden><summary>Search History</summary>
<div id="session-literature-list"></div></details>
<div id="sources-list"></div>
"""


def fulfill(route):
    path = urlsplit(route.request.url).path
    if path == "/":
        route.fulfill(status=200, body=html, content_type="text/html")
    elif path.startswith(("/assets/", "/vendor/")):
        type_by_ext = {".js": "text/javascript", ".css": "text/css", ".woff2": "font/woff2"}
        file = root / path.lstrip("/")
        if file.is_file():
            route.fulfill(path=file, content_type=type_by_ext.get(file.suffix))
        else:
            route.fulfill(status=404)
    elif path == "/api/sessions/s-test/dag":
        route.fulfill(json={"entries": [entry]})
    elif path == "/api/sessions/s-test/evidence":
        route.fulfill(json={"entries": [{"entry_id": "e-1", "support": [
            {"kind": "computation", "status": "completed", "via_entry_id": "e-1", "value": "5/2"}
        ]}]})
    elif path == "/api/sessions/s-test/claim-review":
        assert route.request.post_data_json == {"answer_seq": 12, "claim": "An unlinked statement.", "occurrence": 0, "cited_steps": []}
        route.fulfill(json={"scope": "claim", "verdict": "rejected", "summary": "The selected claim lacks a stated hypothesis."})
    elif path == "/api/sessions/s-test":
        route.fulfill(json={"sources": [], "events": [
            {"kind": "literature.fetched", "payload": {
                "provider": "crossref", "hits": [{
                    "id": "10.1111/2501.00001", "doi": "10.1111/2501.00001", "title": r"Notes on $A_n$",
                }],
            }},
            {"kind": "literature.fetched", "payload": {
                "provider": "arxiv", "papers": [
                    {"id": "2502.00001v2", "title": "Boundary Green functions"},
                ],
            }},
            {"kind": "proof.entry", "payload": {"entry_id": "e-2", "details": "Cite DOI:10.1111/2501.00001."}},
            {"kind": "proof.entry", "payload": {"entry_id": "e-3", "entry_type": "final_answer", "depends_on": ["e-2"]}},
            {"kind": "model.response", "payload": {"stop_reason": "end_turn"}},
            {"kind": "model.text", "payload": {"text": "See DOI:10.1111/2501.00001."}},
            {"kind": "session.end", "payload": {"outcome": "completed"}},
        ]})
    elif path == "/api/sessions/s-other":
        route.fulfill(json={"sources": [], "events": []})
    elif path.startswith("/api/sessions/s-test/"):
        route.fulfill(json={"entries": []})
    else:
        route.fulfill(status=404)


def check_dense_proof_graph(browser):
    """Navigate a real 30-step graph, including its multi-line math and directed edges."""
    entries = [{"entry_id":f"e-{i}", "seq":i, "entry_type":"derivation" if i > 6 else "assumption",
                "summary":rf"At order ${i}$, the approximation to $\frac{{1}}{{1-x}} = \sum_{{n=0}}^{{{i}}} x^n$ converges for $|x|<1$. Its residual is bounded independently.",
                "status":"accepted", "depends_on":[f"e-{i-6}"] if i > 6 else []} for i in range(1, 31)]
    # Reverse wire order to prove keyboard order is determined by dependencies and ledger sequence.
    fixture = html.replace('<div id="dag-viewport"', '''
        <div id="dag-views"><button id="dag-view-graph">Graph</button><button id="dag-view-list">Steps in order</button></div>
        <p id="dag-direction">Arrows point from a prerequisite to the step it supports.</p>
        <ol id="dag-reading-list" class="dag-reading-list"></ol>
        <div id="dag-map"><button id="dag-zoom-fit">Fit</button><button id="dag-zoom-reset">100%</button>
        <div id="dag-viewport" tabindex="0"''').replace('<div id="dag-node-detail">', '</div><div id="dag-node-detail">')
    for width in (1040, 390):
        page = browser.new_page(viewport={"width":width,"height":850})
        try:
            def serve(route):
                path = urlsplit(route.request.url).path
                if path == "/": route.fulfill(body=fixture,content_type="text/html")
                elif path == "/api/sessions/s-test/dag": route.fulfill(json={"entries":list(reversed(entries))})
                else: fulfill(route)
            page.route("**/*", serve)
            page.goto("http://127.0.0.1/")
            page.evaluate("""async () => { const d = await import('/assets/canvas_dag.js');
                d.initDagCanvas(); await d.refreshDag('s-test'); }""")
            ordered = page.locator('#dag-reading-list button')
            expect(ordered).to_have_count(30)
            expect(ordered.first).to_have_attribute("data-entry-id","e-1")
            ordered.first.focus()
            page.keyboard.press("End")
            expect(ordered.last).to_be_focused()
            expect(ordered.last).to_be_in_viewport()
            ordered.last.press("Enter")
            expect(page.locator("#dag-node-detail")).to_be_focused()
            page.get_by_role("button",name="Graph",exact=True).click()
            node = page.locator('.dag-node[data-entry-id="e-30"]')
            expect(node).to_have_attribute("aria-pressed","true")
            expect(node.locator('.dag-node-summary .katex').first).to_be_visible()
            assert "residual" not in node.locator('.dag-node-summary').inner_text()
            assert node.locator('.dag-node-summary').evaluate("e=>e.offsetHeight >= 42 && e.scrollWidth<=e.clientWidth+1")
            edge = page.locator('.dag-edge[data-prerequisite="e-24"][data-consequence="e-30"]')
            expect(edge).to_have_attribute("marker-end","url(#dag-arrow)")
            page.locator("#dag-zoom-fit").click()
            assert int(page.locator("#dag-zoom-reset").inner_text().strip("%")) < 85
            page.locator("#dag-zoom-reset").click()
            expect(node).to_be_in_viewport()
            view = page.locator("#dag-viewport")
            view.focus()
            stage = page.locator(".dag-stage")
            previous = stage.get_attribute("transform")
            page.keyboard.press("ArrowRight")
            expect(stage).not_to_have_attribute("transform",previous)
            node.focus()
            page.keyboard.press("Home")
            expect(page.locator('.dag-node[data-entry-id="e-1"]')).to_be_focused()
            expect(page.locator('.dag-node[data-entry-id="e-1"]')).to_be_in_viewport()
        finally:
            page.close()


def check_research_display(browser):
    check_dense_proof_graph(browser)
    for width in (1440, 390):
        page = browser.new_page(viewport={"width": width, "height": 900})
        errors = []
        page.on("pageerror", lambda error: errors.append(str(error)))
        page.route("**/*", fulfill)
        page.goto("http://127.0.0.1/")
        assert page.evaluate(r"""async () => {
            const dag = await import('/assets/canvas_dag.js');
            const stream = await import('/assets/stream.js');
            const trace = await import('/assets/trace_cards.js');
            dag.initDagCanvas();
            stream.initStream();
            stream.handleThinkingDelta({ delta: 'First $a' });
            stream.handleThinkingDelta({ delta: '^2$ plus \\(b^2\\).' });
            document.body.append(trace.buildStructuredTraceCard({
                tool: 'sympy_eval', status: 'ok',
                output: JSON.stringify({ value: 'Energy $E=mc^2$', latex: 'E=mc^2' }),
            }));
            return await dag.refreshDag('s-test');
        }""") == 1
        math = page.locator(".dag-node-summary .katex")
        expect(math).to_be_visible()
        expect(page.locator(".dag-node-summary .katex-display")).to_have_count(0)
        assert math.bounding_box()["height"] <= page.locator("foreignObject").bounding_box()["height"]
        page.locator(".dag-node").click()
        expect(page.locator("#dag-node-detail .dag-eq-box .katex-display")).to_be_visible()
        expect(page.locator(".thinking-notes .katex")).to_have_count(2)
        expect(page.locator(".sc-result-row .katex")).to_be_visible()
        expect(page.locator(".sc-latex .katex-display")).to_be_visible()
        page.evaluate("""async () => {
            window.sources = await import('/assets/canvas_sources.js');
            sources.initSourcesAndSidetalk({ onSelectStepRef: (id) => { window.selectedSourceStep = id; } });
            await sources.refreshSourcesAndSidetalk('s-test');
            window.stream = await import('/assets/stream.js');
            window.savedMessages = Array.from({ length: 25 }, (_, i) => ({ role: 'user', text: 'Saved step ' + i }));
            stream.renderSessionStream('s-test', { messages: savedMessages });
        }""")
        toggle = page.locator("#session-literature summary")
        expect(toggle).to_have_text("Search History (2)")
        used = page.locator("#used-sources-list")
        expect(used.locator("article")).to_have_count(1)
        expect(used.locator("a.paper-title-link")).to_have_attribute("href", "https://doi.org/10.1111/2501.00001")
        expect(used.get_by_text("Cited in answer")).to_be_visible()
        used.get_by_role("button", name="Proof e-2").click()
        assert page.evaluate("window.selectedSourceStep") == "e-2"
        toggle.click()
        expect(page.locator("#session-literature-list .katex")).to_be_visible()
        expect(page.locator("#session-literature-list a.paper-title-link").last).to_have_attribute(
            "href", "https://doi.org/10.1111/2501.00001"
        )
        page.evaluate("sources.refreshSourcesAndSidetalk('s-other')")
        expect(toggle).to_be_hidden()
        expect(used.locator("article")).to_have_count(0)
        page.evaluate("sources.refreshSourcesAndSidetalk('s-test')")
        expect(toggle).to_have_text("Search History (2)")
        expect(used.locator("article")).to_have_count(1)
        pane = page.locator("#message-stream")
        assert pane.evaluate("(el) => el.scrollHeight - el.clientHeight - el.scrollTop") < 2
        pane.evaluate("el => el.scrollTop = 80")
        page.evaluate("""async () => {
            await new Promise(requestAnimationFrame);
            stream.handleTextDelta({delta: 'Latest streamed output.'});
            await new Promise(requestAnimationFrame);
            stream.appendWarningLine('A warning appeared while reading.');
        }""")
        assert abs(pane.evaluate("el => el.scrollTop") - 80) < 2
        page.evaluate("stream.renderSessionStream('s-test', { messages: savedMessages })")
        assert abs(pane.evaluate("el => el.scrollTop") - 80) < 2
        pane.evaluate("el => el.scrollTop = el.scrollHeight")
        page.evaluate("""async () => {
            await new Promise(requestAnimationFrame);
            stream.handleTextDelta({delta: 'Continued output.'});
            await new Promise(requestAnimationFrame);
        }""")
        assert pane.evaluate("(el) => el.scrollHeight - el.clientHeight - el.scrollTop") < 2
        page.evaluate(r"""async () => {
            const { attachAnswerTrace } = await import('/assets/claim_trace.js');
            stream.initStream({ attachClaimTrace: (row, answer, meta, sid) => attachAnswerTrace(row, answer, meta.seq, sid, {
                inspectStep: (id) => { window.inspectedClaimStep = id; }, openSource: () => {},
            }) });
            stream.renderSessionStream('s-test', { messages: [{ role:'assistant', seq:12,
                text:'The ratio is finite [e-1]. An unlinked statement. See DOI:10.1111/2501.00001.' }] });
        }""")
        page.get_by_role("button", name="Trace a claim").click()
        page.get_by_role("button", name="The ratio is finite [e-1].", exact=True).click()
        selected = page.get_by_role("region", name="Evidence for selected claim")
        expect(selected).to_contain_text("5/2")
        selected.get_by_role("button", name="Open proof step e-1").click()
        assert page.evaluate("window.inspectedClaimStep") == "e-1"
        page.get_by_role("button", name="An unlinked statement.", exact=True).click()
        expect(selected).to_contain_text("No proof step")
        selected.get_by_role("button", name="Review this claim").click()
        expect(selected).to_contain_text("lacks a stated hypothesis")
        page.get_by_role("button", name="See DOI:10.1111/2501.00001.").click()
        expect(selected.get_by_role("link")).to_have_attribute("href", "https://doi.org/10.1111/2501.00001")
        page.get_by_role("button", name="Close claim tracing").click()
        assert not errors, errors
        print(f"{width}px: math, restored paper references, scrollback during streaming, and follow at the bottom passed")
        page.close()
