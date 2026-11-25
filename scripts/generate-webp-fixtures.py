"""Writes WebP test fixtures with libwebp (through Pillow) and prints the
size and FNV-1a 64 hash of the RGBA libwebp decodes each to.

usage: python generate-webp-fixtures.py OUT_DIR
"""
import io
import random
import sys

from PIL import Image, ImageDraw

out_dir = sys.argv[1]
rng = random.Random(1028)


def fnv(data):
    h = 0xcbf29ce484222325
    for b in data:
        h ^= b
        h = (h * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return h


def scene(width, height, alpha=False):
    """Gradients, shapes, text-like strokes and noise: every predictor and
    transform gets used."""
    image = Image.new("RGBA", (width, height))
    pixels = image.load()
    for y in range(height):
        for x in range(width):
            r = (x * 255) // max(1, width - 1)
            g = (y * 255) // max(1, height - 1)
            b = (x * y * 7 + rng.randrange(40)) & 0xFF
            a = 255 if not alpha else max(0, min(255, (x * 9 + y * 5) % 300))
            pixels[x, y] = (r, g, b, a)
    draw = ImageDraw.Draw(image)
    draw.ellipse((width // 5, height // 6, width * 3 // 4, height * 5 // 6), fill=(230, 40, 60, 255 if not alpha else 160))
    draw.rectangle((2, height // 2, width // 3, height - 3), fill=(20, 20, 20, 255))
    for i in range(0, width, 5):
        draw.line((i, 0, width - i, height - 1), fill=(250, 250, 250, 255), width=1)
    return image


def flat(width, height):
    """Few colors, for the color-indexing transform with packed pixels."""
    image = Image.new("RGBA", (width, height), (255, 255, 255, 255))
    draw = ImageDraw.Draw(image)
    draw.rectangle((3, 3, width // 2, height // 2), fill=(0, 120, 215, 255))
    draw.line((0, height - 1, width - 1, 0), fill=(0, 0, 0, 255), width=2)
    return image


fixtures = []


def save(name, image, **options):
    buffer = io.BytesIO()
    image.save(buffer, "WEBP", **options)
    data = buffer.getvalue()
    open(f"{out_dir}/{name}.webp", "wb").write(data)
    decoded = Image.open(io.BytesIO(data))
    decoded.seek(0)
    rgba = decoded.convert("RGBA")
    fixtures.append((name, rgba.size, fnv(rgba.tobytes()), len(data), rgba))


save("lossy", scene(45, 31).convert("RGB"), quality=80)
save("lossy-low", scene(64, 40).convert("RGB"), quality=4, method=6)
save("lossy-alpha", scene(33, 21, alpha=True), quality=70, alpha_quality=100)
save("lossy-alpha-quantized", scene(30, 18, alpha=True), quality=60, alpha_quality=30)
save("lossless", scene(40, 24, alpha=True), lossless=True, quality=100, method=4)
save("lossless-flat", flat(23, 17), lossless=True, quality=100)
save("lossless-fast", scene(29, 13), lossless=True, quality=0, method=0)
frames = [scene(24, 16), flat(24, 16)]
save("animated", frames[0], save_all=True, append_images=frames[1:], duration=100, lossless=True)

for name, size, digest, length, rgba in fixtures:
    corner = rgba.getpixel((0, 0))
    print(f"{name}: {size[0]}x{size[1]} 0x{digest:016x} bytes={length} first={corner}")
