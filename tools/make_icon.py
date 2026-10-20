# Regenerates assets/ from logo.jfif (a navy gauge on a baked-in checkerboard):
#   python tools/make_icon.py logo.jfif assets
# Needs Pillow and NumPy.
import sys
from PIL import Image
import numpy as np

src, out_dir = sys.argv[1], sys.argv[2]
img = np.asarray(Image.open(src).convert("RGB")).astype(np.float32)
lum = img @ np.array([0.299, 0.587, 0.114], dtype=np.float32)
# Background (checkerboard + faint reflections) is light gray/white; the gauge is dark navy.
BG, FG = 215.0, 130.0
alpha = np.clip((BG - lum) / (BG - FG), 0, 1)
# Un-blend edge pixels from the light background so they don't keep a pale fringe.
bg = 240.0
a = np.maximum(alpha, 1e-3)[..., None]
color = np.clip((img - (1 - a) * bg) / a, 0, 255)
rgba = np.dstack([color, alpha * 255]).astype(np.uint8)
im = Image.fromarray(rgba, "RGBA")

ys, xs = np.nonzero(alpha > 0.2)
left, top, right, bottom = xs.min(), ys.min(), xs.max() + 1, ys.max() + 1
im = im.crop((left, top, right, bottom))
side = int(max(im.size) * 1.04)
square = Image.new("RGBA", (side, side), (0, 0, 0, 0))
square.paste(im, ((side - im.width) // 2, (side - im.height) // 2))

big = square.resize((256, 256), Image.LANCZOS)
big.save(f"{out_dir}/logo-256.png")
big.save(f"{out_dir}/compteur.ico", sizes=[(16, 16), (20, 20), (24, 24), (32, 32), (40, 40), (48, 48), (64, 64), (256, 256)])
small = square.resize((64, 64), Image.LANCZOS)
open(f"{out_dir}/icon-64.rgba", "wb").write(small.tobytes())
