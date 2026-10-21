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

# MSIX package images (packaging/Assets) and a 300 px logo for the Store listing.
package_dir = f"{out_dir}/../packaging/Assets"
import os
os.makedirs(package_dir, exist_ok=True)
for name, size in [("StoreLogo", 50), ("Square44x44Logo", 44), ("Square150x150Logo", 150)]:
    square.resize((size, size), Image.LANCZOS).save(f"{package_dir}/{name}.png")
wide = Image.new("RGBA", (310, 150), (0, 0, 0, 0))
wide.alpha_composite(square.resize((120, 120), Image.LANCZOS), ((310 - 120) // 2, 15))
wide.save(f"{package_dir}/Wide310x150Logo.png")
square.resize((300, 300), Image.LANCZOS).save(f"{out_dir}/store-logo-300.png")
