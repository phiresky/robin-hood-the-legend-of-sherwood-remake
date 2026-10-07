"""Assemble the eight rendered jamb views without changing image content."""
from PIL import Image


def assemble(directory):
    views = [Image.open(directory / f"view-{i}-textured.png").convert("RGBA") for i in range(8)]
    size = views[0].size
    assert all(view.size == size for view in views)
    sheet = Image.new("RGBA", (size[0] * 4, size[1] * 2))
    for i, view in enumerate(views):
        sheet.paste(view, ((i % 4) * size[0], (i // 4) * size[1]))
    sheet.save(directory / "textured.png")
