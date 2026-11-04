"""Generate the KenShell app icon source (1024x1024 PNG).

Usage: python3 tools/scripts/gen_icon.py <out.png>
Then: npx tauri icon <out.png>   (writes src-tauri/icons/*)
Design: dark rounded tile, mode-color stripe on the left (identity bar motif), '>_' prompt.
"""
import sys
from PIL import Image, ImageDraw

S = 1024
SS = 4  # supersampling for smooth edges
img = Image.new("RGBA", (S * SS, S * SS), (0, 0, 0, 0))
d = ImageDraw.Draw(img)
k = SS
pad = 72 * k
d.rounded_rectangle([pad, pad, S * k - pad, S * k - pad], radius=190 * k, fill=(20, 23, 29, 255))
# mode stripe (privileged blue)
d.rounded_rectangle([pad + 110 * k, 300 * k, pad + 150 * k, 724 * k], radius=20 * k, fill=(76, 141, 255, 255))
# chevron '>'
w = 64 * k
d.line([(380 * k, 360 * k), (560 * k, 512 * k), (380 * k, 664 * k)], fill=(232, 236, 243, 255), width=w, joint="curve")
for x, y in [(380, 360), (380, 664), (560, 512)]:
    d.ellipse([(x * k - w / 2), (y * k - w / 2), (x * k + w / 2), (y * k + w / 2)], fill=(232, 236, 243, 255))
# underscore '_'
d.rounded_rectangle([600 * k, 632 * k, 800 * k, 696 * k], radius=32 * k, fill=(255, 159, 67, 255))
img = img.resize((S, S), Image.LANCZOS)
img.save(sys.argv[1] if len(sys.argv) > 1 else "icon.png")
