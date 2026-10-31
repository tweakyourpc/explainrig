"""Write the synthetic test documents. Run: python3 tests/fixtures/make_fixtures.py

change-notice.docx copies the *structure* of a real-world change notice (one Heading 1, a bold
"Background:" label, dated paragraphs, bold numbered items whose bullets are a separate list at the
same level, pictures after the bullets) with invented content. procedure.md is a numbered how-to.
"""
import struct
import zipfile
import zlib
from pathlib import Path

HERE = Path(__file__).parent


def png(w, h, rgb):
    raw = b"".join(b"\x00" + bytes(rgb) * w for _ in range(h))
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


W = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" ' \
    'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" ' \
    'xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" ' \
    'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" ' \
    'xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"'


def run(text, bold=False):
    b = "<w:rPr><w:b/></w:rPr>" if bold else ""
    return f'<w:r>{b}<w:t xml:space="preserve">{text}</w:t></w:r>'


def para(*runs, style=None, num=None):
    ppr = ""
    if style or num:
        s = f'<w:pStyle w:val="{style}"/>' if style else ""
        n = f'<w:numPr><w:ilvl w:val="0"/><w:numId w:val="{num}"/></w:numPr>' if num else ""
        ppr = f"<w:pPr>{s}{n}</w:pPr>"
    return f"<w:p>{ppr}{''.join(runs)}</w:p>"


def picture(rid, descr):
    return (f'<w:p><w:r><w:drawing><wp:inline><wp:docPr id="1" name="p" descr="{descr}"/><a:graphic><a:graphicData>'
            f'<pic:pic><pic:blipFill><a:blip r:embed="{rid}"/></pic:blipFill></pic:pic>'
            f'</a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>')


def change_notice():
    body = "".join([
        para(run("Retiring Paper Timesheets - User Experience"), style="Heading1"),
        para(run("Background:", True), run(" The payroll office has announced this change twice. More reminders follow &amp; details are below.")),
        para(run("During this transition time from Mar 3-14, 2027, users that do not have the Timekeeper app configured will be prompted during login to install the Timekeeper app.")),
        para(run("From Mar 15, 2027, onward, paper timesheets will no longer be accepted for any user.")),
        para(run("Users with the Timekeeper app are already set up.", True), style="ListParagraph", num="1"),
        para(run("They will be able to submit hours from any device."), style="ListParagraph", num="2"),
        para(run("They will not have the option to print a paper timesheet."), style="ListParagraph", num="2"),
        picture("rId10", "Timekeeper home screen"),
        para(run("Users without the Timekeeper app already set up. Only have paper timesheets.", True), style="ListParagraph", num="3"),
        para(run("They will be reminded to install the Timekeeper app and can skip the reminder up to 2 times."), style="ListParagraph", num="4"),
        picture("rId11", "Install reminder"),
        para(run("From the staff portal they can set up the Timekeeper app at any time."), style="ListParagraph", num="4"),
    ])
    doc = f'<?xml version="1.0" encoding="UTF-8"?><w:document {W}><w:body>{body}</w:body></w:document>'
    styles = (f'<?xml version="1.0"?><w:styles {W}><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/></w:style>'
              f'<w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/></w:style></w:styles>')
    lvl = lambda fmt: f'<w:lvl w:ilvl="0"><w:numFmt w:val="{fmt}"/></w:lvl>'
    numbering = (f'<?xml version="1.0"?><w:numbering {W}>'
                 f'<w:abstractNum w:abstractNumId="0">{lvl("decimal")}</w:abstractNum>'
                 f'<w:abstractNum w:abstractNumId="1">{lvl("bullet")}</w:abstractNum>'
                 + "".join(f'<w:num w:numId="{n}"><w:abstractNumId w:val="{a}"/></w:num>' for n, a in [(1, 0), (2, 1), (3, 0), (4, 1)])
                 + "</w:numbering>")
    rels = ('<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            '<Relationship Id="rId10" Type="image" Target="media/image1.png"/>'
            '<Relationship Id="rId11" Type="image" Target="media/image2.png"/></Relationships>')
    with zipfile.ZipFile(HERE / "change-notice.docx", "w", zipfile.ZIP_DEFLATED) as z:
        for name, data in [("word/document.xml", doc), ("word/styles.xml", styles), ("word/numbering.xml", numbering),
                           ("word/_rels/document.xml.rels", rels)]:
            z.writestr(zipfile.ZipInfo(name, (2026, 1, 1, 0, 0, 0)), data)
        z.writestr(zipfile.ZipInfo("word/media/image1.png", (2026, 1, 1, 0, 0, 0)), png(40, 30, (40, 90, 200)))
        z.writestr(zipfile.ZipInfo("word/media/image2.png", (2026, 1, 1, 0, 0, 0)), png(30, 50, (240, 150, 0)))


def procedure():
    (HERE / "img").mkdir(exist_ok=True)
    (HERE / "img" / "settings.png").write_bytes(png(60, 40, (20, 160, 90)))
    (HERE / "procedure.md").write_text("""# How to set up the Timekeeper app

You need your staff ID and about five minutes.

1. Download the **Timekeeper** app from the staff portal.
2. Open the app and sign in with your staff ID.
3. Choose **Settings**, then turn on reminders.

   ![The Settings screen](img/settings.png)

4. Submit a test entry to check that it works.
""")


change_notice()
procedure()
print("wrote change-notice.docx, procedure.md, img/settings.png")
