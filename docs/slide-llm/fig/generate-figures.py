#!/usr/bin/env python3
"""LLM戦略の図をSVGとdraw.ioの両形式で生成する。"""
from pathlib import Path
import base64
import html
import io
import xml.etree.ElementTree as ET

from fontTools import subset
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parent
COLORS = {
    "blue": ("#EFF7FF", "#185B80"),
    "red": ("#FFF2F5", "#97283C"),
    "green": ("#F2FBE4", "#668E38"),
    "orange": ("#FFF2E8", "#C46128"),
    "gray": ("#F5F5F5", "#777777"),
}


def node(key, x, y, w, h, text, color="blue", size=22):
    return dict(key=key, x=x, y=y, w=w, h=h, text=text, color=color, size=size)


def edge(points, both=False):
    return dict(points=points, both=both)


FIGURES = {
    "ai-sampling": dict(
        title="新規生成と反復改善", height=280,
        nodes=[
            node("problem", 10, 100, 170, 80, "問題・仕様"),
            node("a", 280, 10, 160, 70, "新規案 A", "orange"),
            node("b", 280, 105, 160, 70, "新規案 B", "green"),
            node("c", 280, 200, 160, 70, "新規案 C", "blue"),
            node("a1", 550, 10, 160, 70, "改善 A₁", "orange"),
            node("b1", 550, 105, 160, 70, "改善 B₁", "green"),
            node("c1", 550, 200, 160, 70, "改善 C₁", "blue"),
            node("a2", 870, 10, 190, 70, "改善 A₂", "orange"),
            node("b2", 870, 105, 190, 70, "改善 B₂", "green"),
            node("c2", 870, 200, 190, 70, "改善 C₂", "blue"),
        ],
        edges=[
            edge([(180, 140), (230, 140), (230, 45), (280, 45)]),
            edge([(180, 140), (280, 140)]),
            edge([(180, 140), (230, 140), (230, 235), (280, 235)]),
            *[edge([(440, y), (550, y)]) for y in [45, 140, 235]],
            *[edge([(710, y), (870, y)]) for y in [45, 140, 235]],
        ],
    ),
    "ai-harness": dict(
        title="人間の考察と自律開発のループ", height=290,
        nodes=[
            node("human", 10, 5, 280, 85, "人間\n考察・議論・評価基準", "red"),
            node("record", 770, 5, 300, 85, "フレームワーク\n記録・継続条件・再開", "gray"),
            node("generate", 10, 150, 220, 80, "LLMが生成・修正", "orange"),
            node("execute", 290, 150, 220, 80, "ビルド・実行"),
            node("evaluate", 570, 150, 220, 80, "採点・比較", "green"),
            node("adopt", 850, 150, 220, 80, "条件を満たせば採用", "red", 20),
        ],
        edges=[
            edge([(120, 90), (120, 150)]),
            edge([(920, 90), (920, 150)]),
            edge([(230, 190), (290, 190)]),
            edge([(510, 190), (570, 190)]),
            edge([(790, 190), (850, 190)]),
            edge([(680, 230), (680, 270), (120, 270), (120, 230)]),
        ],
    ),
    "ai-pool": dict(
        title="ソルバ群を進化させる更新サイクル", height=260,
        nodes=[
            node("pool", 10, 40, 175, 120, "ソルバ群で対戦\n5個の平均順位", size=20),
            node("last", 230, 40, 175, 120, "最下位の枠を\n再構築対象に", "orange"),
            node("new", 450, 40, 175, 120, "新しい開発者\nゼロから構築", "green", 20),
            node("refine", 670, 40, 175, 120, "他4個を超える\nまで評価・改善", "blue", 20),
            node("promote", 890, 40, 180, 120, "単独首位の候補を\nソルバ群に採用", "red", 20),
        ],
        edges=[
            *[edge([(x, 100), (x + 45, 100)]) for x in [185, 405, 625, 845]],
            edge([(980, 160), (980, 225), (98, 225), (98, 160)]),
        ],
    ),
    "ai-test-layers": dict(
        title="単独・対戦・サーバーの評価レイヤー", height=230,
        nodes=[
            node("solo", 10, 20, 300, 180, "単独テスタ：solo\n1チームで実行\n経路・補給・得点", "blue", 25),
            node("game", 390, 20, 300, 180, "対戦テスタ：game\n5チームを同時に実行\n相手・在庫・混雑", "green", 25),
            node("server", 770, 20, 300, 180, "サーバーテスタ：server\n複数PCのソルバが参加\n通信・提出・期限", "red", 24),
        ],
        edges=[
            edge([(310, 110), (390, 110)]),
            edge([(690, 110), (770, 110)]),
        ],
    ),
    "ai-distributed": dict(
        title="3台で77探索と3制御を動かす構成", height=285,
        nodes=[
            node("pc2", 10, 10, 300, 140, "PC 2：16T\n探索15 ＋ 制御1\nworker", "green", 25),
            node("pc1", 390, 10, 300, 140, "PC 1：32T\n探索31 ＋ 制御1\ncoordinator", "orange", 25),
            node("pc3", 770, 10, 300, 140, "PC 3：32T\n探索31 ＋ 制御1\nworker", "blue", 25),
            node("submit", 390, 210, 300, 70, "PC 1で集約・提出", "red", 25),
        ],
        edges=[
            edge([(310, 80), (390, 80)], both=True),
            edge([(690, 80), (770, 80)], both=True),
            edge([(540, 150), (540, 210)]),
        ],
    ),
    "ai-knowledge": dict(
        title="実装の再構築と知見の回収を組み合わせる構想", height=280,
        nodes=[
            node("trial", 10, 10, 260, 90, "独立した案で\n実装・実験する", "orange"),
            node("check", 410, 10, 260, 90, "成績・リプレイから\n性質と失敗を検証", "blue"),
            node("knowledge", 810, 10, 260, 90, "検証した知見を\n文書として蓄積", "green"),
            node("spec", 10, 185, 260, 80, "問題・公開仕様", "gray"),
            node("new", 410, 185, 260, 80, "次の新規設計", "red"),
        ],
        edges=[
            edge([(270, 55), (410, 55)]),
            edge([(670, 55), (810, 55)]),
            edge([(940, 100), (940, 225), (670, 225)]),
            edge([(270, 225), (410, 225)]),
        ],
    ),
}


def font_data():
    chars = "".join(n["text"] for f in FIGURES.values() for n in f["nodes"])
    font = TTFont(ROOT.parent / "fonts" / "Murecho-wght.ttf")
    options = subset.Options()
    options.flavor = "woff2"
    sub = subset.Subsetter(options=options)
    sub.populate(text=chars)
    sub.subset(font)
    font.flavor = "woff2"
    buf = io.BytesIO()
    font.save(buf)
    return base64.b64encode(buf.getvalue()).decode()


def svg(name, fig, font):
    parts = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="1080" height="{fig["height"]}" viewBox="0 0 1080 {fig["height"]}" role="img">',
        f'<title>{html.escape(fig["title"])}</title>',
        '<defs><style>@font-face{font-family:Murecho;src:url(data:font/woff2;base64,'
        + font + ') format("woff2")}text{font-family:Murecho,sans-serif;fill:#242424}</style>',
        '<marker id="arrow" markerWidth="9" markerHeight="9" refX="8" refY="4.5" orient="auto-start-reverse" markerUnits="userSpaceOnUse"><path d="M0,0 L9,4.5 L0,9 Z" fill="#666"/></marker></defs>',
    ]
    for e in fig["edges"]:
        points = " ".join(f"{x},{y}" for x, y in e["points"])
        start = ' marker-start="url(#arrow)"' if e["both"] else ""
        parts.append(f'<polyline points="{points}" fill="none" stroke="#666" stroke-width="2.3" marker-end="url(#arrow)"{start}/>')
    for n in fig["nodes"]:
        fill, stroke = COLORS[n["color"]]
        parts.append(f'<rect x="{n["x"]}" y="{n["y"]}" width="{n["w"]}" height="{n["h"]}" rx="12" fill="{fill}" stroke="{stroke}" stroke-width="2"/>')
        lines = n["text"].split("\n")
        line_height = n["size"] * 1.45
        baseline = n["y"] + n["h"] / 2 - (len(lines) - 1) * line_height / 2 + n["size"] * 0.36
        for i, line in enumerate(lines):
            parts.append(f'<text x="{n["x"] + n["w"]/2}" y="{baseline + i*line_height:.2f}" text-anchor="middle" font-size="{n["size"]}" font-weight="500">{html.escape(line)}</text>')
    parts.append("</svg>")
    (ROOT / f"{name}.svg").write_text("\n".join(parts) + "\n")


def drawio(name, fig):
    file = ET.Element("mxfile", host="app.diagrams.net")
    diagram = ET.SubElement(file, "diagram", id=name, name=fig["title"])
    model = ET.SubElement(diagram, "mxGraphModel", grid="1", gridSize="10", page="1", pageScale="1", pageWidth="1080", pageHeight=str(fig["height"]))
    root = ET.SubElement(model, "root")
    ET.SubElement(root, "mxCell", id="0")
    ET.SubElement(root, "mxCell", id="1", parent="0")
    for n in fig["nodes"]:
        fill, stroke = COLORS[n["color"]]
        style = f"rounded=1;whiteSpace=wrap;html=1;fillColor={fill};strokeColor={stroke};strokeWidth=2;fontColor=#242424;fontFamily=Murecho;fontSize={n['size']};align=center;verticalAlign=middle;spacing=8;"
        cell = ET.SubElement(root, "mxCell", id=n["key"], value=n["text"].replace("\n", "<br>"), style=style, vertex="1", parent="1")
        ET.SubElement(cell, "mxGeometry", x=str(n["x"]), y=str(n["y"]), width=str(n["w"]), height=str(n["h"]), attrib={"as": "geometry"})
    for i, e in enumerate(fig["edges"]):
        style = "edgeStyle=none;html=1;strokeColor=#666666;strokeWidth=2.3;endArrow=block;endFill=1;"
        if e["both"]:
            style += "startArrow=block;startFill=1;"
        cell = ET.SubElement(root, "mxCell", id=f"edge{i}", edge="1", parent="1", style=style)
        geo = ET.SubElement(cell, "mxGeometry", relative="1", attrib={"as": "geometry"})
        for label, point in [("sourcePoint", e["points"][0]), ("targetPoint", e["points"][-1])]:
            ET.SubElement(geo, "mxPoint", x=str(point[0]), y=str(point[1]), attrib={"as": label})
        if len(e["points"]) > 2:
            arr = ET.SubElement(geo, "Array", attrib={"as": "points"})
            for x, y in e["points"][1:-1]:
                ET.SubElement(arr, "mxPoint", x=str(x), y=str(y))
    ET.indent(file, space="  ")
    ET.ElementTree(file).write(ROOT / f"{name}.drawio", encoding="utf-8", xml_declaration=True)


if __name__ == "__main__":
    font = font_data()
    for name, fig in FIGURES.items():
        svg(name, fig, font)
        drawio(name, fig)
    print(f"Generated {len(FIGURES)} SVG/draw.io pairs.")
