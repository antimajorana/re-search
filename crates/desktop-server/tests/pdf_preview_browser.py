"""Verify original PDF authorization, physical page anchors and pixels in the offline viewer."""
import base64
from pathlib import Path
import tempfile
from urllib.parse import urlencode, urlsplit

from playwright.sync_api import expect
from recovery_example_browser import open_install


def marked_pdf():
    pages = [
        b"1 0 0 rg 0 0 100 100 re f BT /F1 9 Tf 5 75 Td (--- [Page 3] ---) Tj ET",
        b"0 0 1 rg 0 0 100 100 re f",
        b"0 1 0 rg 0 0 100 100 re f BT /F1 9 Tf 5 75 Td (Figure 5. Green rectangle) Tj ET",
    ]
    objects = [b"<< /Type /Catalog /Pages 2 0 R >>",
               b"<< /Type /Pages /Kids [4 0 R 6 0 R 8 0 R] /Count 3 >>",
               b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"]
    for i, content in enumerate(pages):
        page = (f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources "
                f"<< /Font << /F1 3 0 R >> >> /Contents {5+2*i} 0 R >>")
        objects.append(page.encode())
        objects.append(f"<< /Length {len(content)} >>\nstream\n".encode() + content + b"\nendstream")
    data, offsets = bytearray(b"%PDF-1.4\n"), [0]
    for i, obj in enumerate(objects, 1):
        offsets.append(len(data))
        data.extend(f"{i} 0 obj\n".encode() + obj + b"\nendobj\n")
    xref = len(data)
    data.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode())
    for offset in offsets[1:]:
        data.extend(f"{offset:010d} 00000 n \n".encode())
    data.extend(f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode())
    return bytes(data)


def check_pdf_preview(browser, binary):
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "paper").mkdir()
        with open_install(browser, binary, root) as page:
            sid, other, token = page.evaluate("""async () => { const api = await import('/assets/api.js');
                return [(await api.createSession({})).id,(await api.createSession({})).id,api.getCsrfToken()]; }""")
            server = f"{urlsplit(page.url).scheme}://{urlsplit(page.url).netloc}"
            headers = {"X-ReSearch-Token": token}
            data = marked_pdf()
            uploaded = page.request.post(f"{server}/api/sessions/{sid}/upload", headers=headers,
                data={"files":[{"name":"panels.pdf","content_base64":base64.b64encode(data).decode()}]})
            assert uploaded.ok, uploaded.text()
            source = uploaded.json()["ingested"][0]
            path, source_id = source["primary_path"], source["source_id"]
            lines = (root / "data" / "sessions" / sid / "sources" / path).read_text(encoding="utf-8").splitlines()
            line = next(i for i, text in enumerate(lines, 1) if "Green rectangle" in text)
            endpoint = f"{server}/api/sessions/{sid}/sources/{source_id}/original-pdf"
            assert page.request.get(endpoint).status == 401
            assert page.request.get(endpoint.replace(f"/{sid}/", f"/{other}/"), headers=headers).status == 404
            first = page.request.get(endpoint + "?" + urlencode({"path":path,"line":2}), headers=headers)
            assert first.headers["x-research-pdf-page"] == "1" and first.body() == data
            page.evaluate("""async args => { const {openOriginalPdf} = await import('/assets/pdf_preview.js');
                await openOriginalPdf(args); }""", {"sessionId":sid,"sourceId":source_id,"path":path,"startLine":line,"label":"Figure 5"})
            dialog = page.get_by_role("dialog", name="Original PDF")
            def check_color(number, channel):
                canvas = dialog.get_by_role("img", name=f"Original PDF page {number} of 3")
                expect(canvas).to_be_visible()
                color = canvas.evaluate("c => [...c.getContext('2d').getImageData(c.width/2,c.height/2,1,1).data]")
                assert color[channel] >= 250 and sum(color[:3]) <= 260, color
            check_color(3, 1)
            dialog.get_by_role("button", name="Previous page").click()
            check_color(2, 2)
            dialog.get_by_role("button", name="Previous page").click()
            check_color(1, 0)
            dialog.get_by_role("button", name="Close").click()
            expect(dialog).to_have_count(0)
