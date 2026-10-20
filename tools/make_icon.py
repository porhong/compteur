# Regenerates assets/ from logo.png (a rounded-square app tile on a transparent background):
#   python tools/make_icon.py logo.png assets
# Needs Pillow and NumPy.
import sys
from PIL import Image
import numpy as np

# Tile corner radius as a share of its width; pixels outside the rounded tile (stray glow) are cut.
CORNER = 0.13

src, out_dir = sys.argv[1], sys.argv[2]
rgba = np.asarray(Image.open(src).convert("RGBA")).astype(np.float32)
ys, xs = np.nonzero(rgba[..., 3] > 240)
left, top, right, bottom = xs.min(), ys.min(), xs.max() + 1, ys.max() + 1
tile = rgba[top:bottom, left:right]

h, w = tile.shape[:2]
r = CORNER * w
fy, fx = np.mgrid[0:h, 0:w] + 0.5
dx = np.maximum(np.maximum(r - fx, fx - (w - r)), 0)
dy = np.maximum(np.maximum(r - fy, fy - (h - r)), 0)
inside = np.clip(r - np.hypot(dx, dy) + 0.5, 0, 1)  # 1 px anti-aliased edge
tile[..., 3] *= inside

side = max(w, h)
square = Image.new("RGBA", (side, side), (0, 0, 0, 0))
square.paste(Image.fromarray(tile.astype(np.uint8), "RGBA"), ((side - w) // 2, (side - h) // 2))

big = square.resize((256, 256), Image.LANCZOS)
big.save(f"{out_dir}/logo-256.png")
big.save(f"{out_dir}/compteur.ico", sizes=[(16, 16), (20, 20), (24, 24), (32, 32), (40, 40), (48, 48), (64, 64), (256, 256)])
small = square.resize((64, 64), Image.LANCZOS)
open(f"{out_dir}/icon-64.rgba", "wb").write(small.tobytes())
