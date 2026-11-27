#!/usr/bin/env python3
"""Generates the placeholder ChesLan piece set (original art, CC0) into ui/assets/pieces/.
Run: python3 scripts/gen_pieces.py   (needs Pillow). Replace the PNGs with your own any time."""
from PIL import Image, ImageDraw, ImageFilter
import pathlib

OUT = pathlib.Path(__file__).resolve().parent.parent / "ui/assets/pieces"
SS, FINAL = 1024, 256
K = SS / 100.0  # drawing units: 0..100

def P(pts): return [(x * K, y * K) for x, y in pts]
def R(b): return [v * K for v in b]

# shape = (kind, data); kinds: poly, ell, rrect
def base():
    return [("rrect", (22, 82, 78, 92, 3.5)), ("rrect", (28, 75, 72, 83, 3))]

PIECES = {
    "pawn": base() + [("ell", (38, 14, 62, 38)), ("rrect", (36, 38, 64, 45, 3)),
                      ("poly", [(41, 44), (59, 44), (66, 76), (34, 76)])],
    "rook": base() + [("rrect", (29, 12, 41, 27, 1.5)), ("rrect", (44, 12, 56, 27, 1.5)), ("rrect", (59, 12, 71, 27, 1.5)),
                      ("rrect", (29, 24, 71, 32, 2)), ("poly", [(35, 31), (65, 31), (63, 70), (37, 70)]),
                      ("rrect", (29, 66, 71, 77, 3))],
    "knight": base() + [("poly", [(28, 78), (34, 58), (41, 47), (30, 45), (23, 40), (21, 35), (27, 27), (36, 20),
                                   (39, 9), (46, 17), (58, 17), (70, 32), (76, 54), (72, 78)])],
    "bishop": base() + [("ell", (40, 14, 60, 44)), ("ell", (45.5, 7, 54.5, 16)), ("rrect", (37, 43, 63, 50, 3)),
                        ("poly", [(42, 49), (58, 49), (65, 76), (35, 76)])],
    "queen": base() + [("poly", [(33, 68), (25, 27), (37, 45), (39, 21), (46, 43), (50, 17), (54, 43), (61, 21),
                                  (63, 45), (75, 27), (67, 68)]),
                       ("ell", (21.5, 20, 30.5, 29)), ("ell", (34.5, 13, 43.5, 22)), ("ell", (45.5, 9, 54.5, 18)),
                       ("ell", (56.5, 13, 65.5, 22)), ("ell", (69.5, 20, 78.5, 29)), ("rrect", (32, 64, 68, 72, 3))],
    "king": base() + [("rrect", (47, 3, 53, 24, 1.5)), ("rrect", (41, 9, 59, 15, 1.5)),
                      ("ell", (30, 24, 70, 60)), ("poly", [(35, 52), (65, 52), (69, 76), (31, 76)]),
                      ("rrect", (32, 60, 68, 69, 3))],
}

# detail strokes drawn on top: (kind, data)
DETAILS = {
    "pawn": [], "rook": [("line", (37, 31, 63, 31)), ("line", (31, 66, 69, 66))],
    "knight": [("dot", (40, 29, 2.4)), ("dot", (26, 38, 1.4)), ("line", (50, 24, 66, 40))],
    "bishop": [("line", (55, 19, 47, 33)), ("line", (38, 46, 62, 46))],
    "queen": [("line", (33, 64, 67, 64))], "king": [("line", (32, 60, 68, 60)), ("line", (33, 66, 67, 66))],
}

STYLE = {
    "white": dict(hi=(250, 247, 238), lo=(196, 188, 168), line=(28, 40, 46), detail=(28, 40, 46)),
    "black": dict(hi=(92, 108, 118), lo=(30, 40, 46), line=(8, 14, 18), detail=(205, 215, 220)),
}

def render(name, color):
    s = STYLE[color]
    mask = Image.new("L", (SS, SS), 0)
    d = ImageDraw.Draw(mask)
    for kind, data in PIECES[name]:
        if kind == "poly": d.polygon(P(data), fill=255)
        elif kind == "ell": d.ellipse(R(data), fill=255)
        else: d.rounded_rectangle(R(data[:4]), radius=data[4] * K, fill=255)
    outline = mask.filter(ImageFilter.MaxFilter(23))  # ~2.2 units of stroke
    # horizontal gradient for a little volume
    grad = Image.linear_gradient("L").rotate(90).resize((SS, SS))  # white->black left to right? normalise below
    grad = grad.transpose(Image.FLIP_LEFT_RIGHT)
    fill = Image.composite(Image.new("RGB", (SS, SS), s["lo"]), Image.new("RGB", (SS, SS), s["hi"]), grad.point(lambda v: int(v * 0.9)))
    img = Image.new("RGBA", (SS, SS), (0, 0, 0, 0))
    img.paste(Image.new("RGBA", (SS, SS), s["line"] + (255,)), mask=outline)
    img.paste(fill.convert("RGBA"), mask=mask)
    dd = ImageDraw.Draw(img)
    w = int(1.3 * K)
    for kind, data in DETAILS[name]:
        if kind == "line":
            dd.line([data[0] * K, data[1] * K, data[2] * K, data[3] * K], fill=s["detail"], width=w)
        else:
            x, y, r = data
            dd.ellipse([(x - r) * K, (y - r) * K, (x + r) * K, (y + r) * K], fill=s["detail"])
    return img.resize((FINAL, FINAL), Image.LANCZOS)

if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    for color in STYLE:
        for name in PIECES:
            render(name, color).save(OUT / f"{color}{name}.png")
    print("wrote", len(STYLE) * len(PIECES), "pieces to", OUT)
