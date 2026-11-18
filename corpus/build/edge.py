"""Writes corpus/edge.docx: WordprocessingML written by hand for the cases
that Word does not write from its interface or writes only one way.

    python3 edge.py OUT.docx BIRD.png

Fields nested in field codes, hidden text, symbols and special hyphens,
breaks, joined paragraphs, mc:AlternateContent whose choice Word does not
understand, an ignorable element of an unknown namespace, custom XML and
smart tags, a table row with grid columns before and after, an equation, a
group content control and one that shows its placeholder, a note with its
own mark, a bookmark over two paragraphs, a legacy VML picture, two
paragraphs that share a paraId, a vertical merge that continues in a row
with grid columns before it, a text box in both branches of
mc:AlternateContent, content controls around a row and around a cell, and
toggle properties through a chain of character styles, a table style and
a character style that sets bold off.
"""
import sys
import zipfile

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
NS = (
    f'xmlns:w="{W}" '
    'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" '
    'xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" '
    'xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" '
    'xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" '
    'xmlns:v="urn:schemas-microsoft-com:vml" '
    'xmlns:o="urn:schemas-microsoft-com:office:office" '
    'xmlns:x="urn:example:field-notes" '
    'xmlns:future="urn:example:future-word" '
    'xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" '
    'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" '
    'xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" '
    'mc:Ignorable="w14 x"'
)
DECL = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
AUTHOR = 'w:author="Wren Ashby" w:date="2026-05-03T08:00:00Z"'


def p(body, pid=None, ppr=""):
    attr = f' w14:paraId="{pid}" w14:textId="77777777"' if pid else ""
    return f"<w:p{attr}>{ppr}{body}</w:p>"


def r(text, rpr=""):
    space = ' xml:space="preserve"' if text != text.strip() else ""
    return f"<w:r>{rpr}<w:t{space}>{text}</w:t></w:r>"


def fld(kind):
    return f'<w:r><w:fldChar w:fldCharType="{kind}"/></w:r>'


def instr(code):
    return f'<w:r><w:instrText xml:space="preserve">{code}</w:instrText></w:r>'


def style(sid):
    return f'<w:pPr><w:pStyle w:val="{sid}"/></w:pPr>'


body = "".join([
    p(r("Edge cases of the Alder Marshes survey"), "10000001", style("Title")),
    p(r("Fields"), "10000002", style("Heading1")),
    # A complex field nested in the code of another.
    p(r("The sum is ") + fld("begin") + instr(" IF ") + fld("begin") + instr(" = 2 + 2 ")
      + fld("separate") + r("4") + fld("end") + instr(' = 4 "right" "wrong" ')
      + fld("separate") + r("right") + fld("end") + r("."), "10000003"),
    # A field without a separator has no result.
    p(r("Nothing follows: ") + fld("begin") + instr(" PAGE ") + fld("end") + r("."), "10000004"),
    p(r("Text"), "10000005", style("Heading1")),
    p(r("Seen ") + r("unseen ", "<w:rPr><w:vanish/></w:rPr>") + r("and seen again."), "10000006"),
    p(r("Tick") + '<w:r><w:sym w:font="Wingdings" w:char="F0FC"/></w:r>'
      + r(" and non") + "<w:r><w:noBreakHyphen/></w:r>" + r("breaking, opt")
      + "<w:r><w:softHyphen/></w:r>" + r("ional.") + "<w:r><w:tab/></w:r>" + r("After a tab.")
      + '<w:r><w:br/></w:r>' + r("After a line break."), "10000007"),
    p(r("Before a page break.") + '<w:r><w:br w:type="page"/></w:r>' + r("After it."), "1000001D"),
    # The mark of this paragraph is deleted, so it joins the next one.
    p(r("Joined"), "10000008", f"<w:pPr><w:rPr><w:del w:id=\"901\" {AUTHOR}/></w:rPr></w:pPr>"),
    p(r(" together."), "10000009"),
    # Word does not understand the namespace the choice requires.
    p('<mc:AlternateContent><mc:Choice Requires="future">' + r("Choice text.")
      + "</mc:Choice><mc:Fallback>" + r("Fallback text.") + "</mc:Fallback></mc:AlternateContent>",
      "1000000A"),
    # An ignorable element of an unknown namespace: its content is ignored.
    p(r("Before. ") + "<x:aside>" + r("Inside an aside.") + "</x:aside>" + r("After."), "1000000B"),
    '<w:customXml w:uri="urn:example:field-notes" w:element="note">'
    + p(r("In custom XML."), "1000000C") + "</w:customXml>",
    p(r("Near ") + '<w:smartTag w:uri="urn:schemas-microsoft-com:office:smarttags" w:element="place">'
      + r("Alder") + "</w:smartTag>" + r(" weir."), "1000000D"),
    # An equation.
    p(r("Pairs: ") + "<m:oMath><m:r><m:t>a+b=c</m:t></m:r></m:oMath>", "1000000E"),
    # A bookmark over two paragraphs.
    p('<w:bookmarkStart w:id="1" w:name="two_paragraphs"/>' + r("First half,"), "1000000F"),
    p(r("second half.") + '<w:bookmarkEnd w:id="1"/>', "10000010"),
    # Two paragraphs share a paraId.
    p(r("Twin one."), "10000011"),
    p(r("Twin two."), "10000011"),
    p(r("A direct outline level."), "10000012", '<w:pPr><w:outlineLvl w:val="1"/></w:pPr>'),
    p(r("A note with its own mark") + '<w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr>'
      '<w:footnoteReference w:customMarkFollows="1" w:id="2"/><w:t>*</w:t></w:r>'
      + r(" and one with a number") + '<w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr>'
      '<w:footnoteReference w:id="1"/></w:r>' + r("."), "10000013"),
    p(r("Tables and controls"), "10000014", style("Heading1")),
    '<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/></w:tblPr>'
    '<w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>'
    '<w:tr><w:tc>' + p(r("Wren"), "10000015") + '</w:tc><w:tc>' + p(r("Robin"), "10000016")
    + '</w:tc><w:tc>' + p(r("Dunnock"), "10000017") + '</w:tc></w:tr>'
    '<w:tr><w:trPr><w:gridBefore w:val="1"/><w:gridAfter w:val="1"/></w:trPr><w:tc>'
    + p(r("Middle only"), "10000018") + '</w:tc></w:tr></w:tbl>',
    p("", "10000019"),
    '<w:sdt><w:sdtPr><w:alias w:val="Visit"/><w:tag w:val="visit:group"/><w:group/></w:sdtPr>'
    '<w:sdtContent>' + p(r("Visit on ") + '<w:sdt><w:sdtPr><w:alias w:val="Day"/>'
    '<w:tag w:val="visit:day"/><w:showingPlcHdr/><w:text/></w:sdtPr><w:sdtContent>'
    + r("Click to enter the day.", '<w:rPr><w:rStyle w:val="PlaceholderText"/></w:rPr>')
    + "</w:sdtContent></w:sdt>", "1000001A") + p(r("Second line of the visit."), "1000001B")
    + "</w:sdtContent></w:sdt>",
    p(r("More tables, controls and styles"), "10000020", style("Heading1")),
    # Column 1 merges down into a row whose first grid column is skipped.
    '<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/></w:tblPr>'
    '<w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>'
    '<w:tr><w:tc>' + p(r("Site"), "10000021") + '</w:tc><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr>'
    + p(r("Reed beds"), "10000022") + '</w:tc><w:tc>' + p(r("12"), "10000023") + '</w:tc></w:tr>'
    '<w:tr><w:trPr><w:gridBefore w:val="1"/></w:trPr><w:tc><w:tcPr><w:vMerge/></w:tcPr>'
    + p("", "10000024") + '</w:tc><w:tc>' + p(r("9"), "10000025") + '</w:tc></w:tr></w:tbl>',
    p("", "10000026"),
    # A text box in both branches of mc:AlternateContent: Word reads one.
    p(r("Beside a box. ") + '<w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing>'
      '<wp:anchor distT="0" distB="0" distL="114300" distR="114300" simplePos="0" relativeHeight="1" '
      'behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/>'
      '<wp:positionH relativeFrom="column"><wp:posOffset>3000000</wp:posOffset></wp:positionH>'
      '<wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV>'
      '<wp:extent cx="1270000" cy="508000"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapSquare wrapText="bothSides"/>'
      '<wp:docPr id="7" name="Box"/><wp:cNvGraphicFramePr/><a:graphic><a:graphicData '
      'uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:cNvSpPr txBox="1"/>'
      '<wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1270000" cy="508000"/></a:xfrm>'
      '<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></wps:spPr><wps:txbx><w:txbxContent>'
      + p(r("Box text, read once."), "10000027") + '</w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp>'
      '</a:graphicData></a:graphic></wp:anchor></w:drawing></mc:Choice><mc:Fallback><w:pict>'
      '<v:shape id="box" style="position:absolute;width:100pt;height:40pt" type="#_x0000_t202">'
      '<v:textbox><w:txbxContent>' + p(r("Box text, read once."), "10000028")
      + '</w:txbxContent></v:textbox></v:shape></w:pict></mc:Fallback></mc:AlternateContent></w:r>',
      "10000029"),
    # Content controls around a row and around a cell.
    '<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/></w:tblPr>'
    '<w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>'
    '<w:tr><w:tc>' + p(r("Wren"), "1000002A") + '</w:tc><w:tc>' + p(r("4"), "1000002B") + '</w:tc></w:tr>'
    '<w:sdt><w:sdtPr><w:alias w:val="Row"/><w:tag w:val="visit:row"/></w:sdtPr><w:sdtContent>'
    '<w:tr><w:tc>' + p(r("Robin"), "1000002C") + '</w:tc>'
    '<w:sdt><w:sdtPr><w:alias w:val="Cell"/><w:tag w:val="visit:cell"/><w:text/></w:sdtPr><w:sdtContent>'
    '<w:tc>' + p(r("2"), "1000002D") + '</w:tc></w:sdtContent></w:sdt></w:tr></w:sdtContent></w:sdt></w:tbl>',
    p("", "1000002E"),
    # Bold through a chain of character styles that both set it.
    p('<w:r><w:rPr><w:rStyle w:val="BoldChild"/></w:rPr><w:t>Bold through a chain.</w:t></w:r>', "1000002F"),
    # A character style that sets bold off, in a bold heading.
    p('<w:r><w:rPr><w:rStyle w:val="NotBold"/></w:rPr><w:t>Bold off in a heading.</w:t></w:r>', "10000030",
      style("Heading1")),
    # Bold from a table style and from a paragraph style.
    '<w:tbl><w:tblPr><w:tblStyle w:val="BoldTable"/><w:tblW w:w="0" w:type="auto"/></w:tblPr>'
    '<w:tblGrid><w:gridCol w:w="3000"/></w:tblGrid><w:tr><w:tc>'
    + p(r("Bold table, bold paragraph."), "10000031", style("BoldPara")) + '</w:tc></w:tr><w:tr><w:tc>'
    + p(r("Bold table, plain paragraph."), "10000032") + '</w:tc></w:tr></w:tbl>',
    p("", "10000033"),
    p(r("A legacy picture: ") + '<w:r><w:pict><v:shape id="bird" type="#_x0000_t75" '
      'style="width:48pt;height:32pt" alt="A bird shape drawn the old way">'
      '<v:imagedata r:id="rId4" o:title="Legacy bird"/></v:shape></w:pict></w:r>', "1000001C"),
    '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1417" w:right="1417" '
    'w:bottom="1134" w:left="1417" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>',
])

document = f"{DECL}<w:document {NS}><w:body>{body}</w:body></w:document>"

styles = f"""{DECL}<w:styles xmlns:w="{W}">
<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri"/><w:sz w:val="22"/></w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:rPr><w:sz w:val="48"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="FootnoteText"><w:name w:val="footnote text"/><w:basedOn w:val="Normal"/></w:style>
<w:style w:type="character" w:default="1" w:styleId="DefaultParagraphFont"><w:name w:val="Default Paragraph Font"/></w:style>
<w:style w:type="character" w:styleId="FootnoteReference"><w:name w:val="footnote reference"/><w:rPr><w:vertAlign w:val="superscript"/></w:rPr></w:style>
<w:style w:type="character" w:styleId="PlaceholderText"><w:name w:val="Placeholder Text"/><w:rPr><w:color w:val="808080"/></w:rPr></w:style>
<w:style w:type="character" w:styleId="BoldBase"><w:name w:val="Bold Base"/><w:rPr><w:b/></w:rPr></w:style>
<w:style w:type="character" w:styleId="BoldChild"><w:name w:val="Bold Child"/><w:basedOn w:val="BoldBase"/><w:rPr><w:b/></w:rPr></w:style>
<w:style w:type="character" w:styleId="NotBold"><w:name w:val="Not Bold"/><w:rPr><w:b w:val="0"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="BoldPara"><w:name w:val="Bold Para"/><w:basedOn w:val="Normal"/><w:rPr><w:b/></w:rPr></w:style>
<w:style w:type="table" w:styleId="BoldTable"><w:name w:val="Bold Table"/><w:basedOn w:val="TableNormal"/><w:rPr><w:b/></w:rPr></w:style>
<w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/></w:style>
<w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/><w:basedOn w:val="TableNormal"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:left w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:bottom w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:right w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:insideV w:val="single" w:sz="4" w:space="0" w:color="auto"/></w:tblBorders></w:tblPr></w:style>
</w:styles>"""

footnotes = f"""{DECL}<w:footnotes xmlns:w="{W}">
<w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote>
<w:footnote w:type="continuationSeparator" w:id="0"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>
<w:footnote w:id="1"><w:p><w:pPr><w:pStyle w:val="FootnoteText"/></w:pPr><w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr><w:footnoteRef/></w:r><w:r><w:t xml:space="preserve"> Counted twice.</w:t></w:r></w:p></w:footnote>
<w:footnote w:id="2"><w:p><w:pPr><w:pStyle w:val="FootnoteText"/></w:pPr><w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr><w:t>*</w:t></w:r><w:r><w:t xml:space="preserve"> Marked by hand.</w:t></w:r></w:p></w:footnote>
</w:footnotes>"""

settings = f"""{DECL}<w:settings xmlns:w="{W}"><w:defaultTabStop w:val="708"/><w:footnotePr><w:footnote w:id="-1"/><w:footnote w:id="0"/></w:footnotePr></w:settings>"""

types = f"""{DECL}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Default Extension="png" ContentType="image/png"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
<Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>
<Override PartName="/word/footnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"/>
<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>
<Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>
</Types>"""

REL = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
package_rels = f"""{DECL}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="{REL}/officeDocument" Target="word/document.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>
<Relationship Id="rId3" Type="{REL}/extended-properties" Target="docProps/app.xml"/>
</Relationships>"""

document_rels = f"""{DECL}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="{REL}/styles" Target="styles.xml"/>
<Relationship Id="rId2" Type="{REL}/settings" Target="settings.xml"/>
<Relationship Id="rId3" Type="{REL}/footnotes" Target="footnotes.xml"/>
<Relationship Id="rId4" Type="{REL}/image" Target="media/bird.png"/>
</Relationships>"""

core = f"""{DECL}<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title>Edge cases</dc:title><dc:creator>Wren Ashby</dc:creator><cp:lastModifiedBy>Wren Ashby</cp:lastModifiedBy><dcterms:created xsi:type="dcterms:W3CDTF">2026-05-03T08:00:00Z</dcterms:created></cp:coreProperties>"""

app = f"""{DECL}<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>tabellion corpus</Application></Properties>"""

out, png = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    def add(name, data):
        info = zipfile.ZipInfo(name, date_time=(2026, 5, 3, 8, 0, 0))
        info.compress_type = zipfile.ZIP_DEFLATED
        z.writestr(info, data)
    add("[Content_Types].xml", types)
    add("_rels/.rels", package_rels)
    add("docProps/core.xml", core)
    add("docProps/app.xml", app)
    add("word/document.xml", document)
    add("word/_rels/document.xml.rels", document_rels)
    add("word/styles.xml", styles)
    add("word/settings.xml", settings)
    add("word/footnotes.xml", footnotes)
    add("word/media/bird.png", open(png, "rb").read())
