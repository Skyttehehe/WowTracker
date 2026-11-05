# Map images (optional)

Without images the site draws a schematic map: one box per zone, placed with the game's own
zone bounds (`../data/maps.json`). To show the real map art, put images here named by
uiMapID:

| File | Shows |
|---|---|
| `1414.jpg` | Kalimdor (continent view) |
| `1415.jpg` | Eastern Kingdoms (continent view) |
| `1429.jpg` | Elwynn Forest (zone view) |
| ... | any uiMapID in `../data/maps.json` |

`.webp`, `.jpg` and `.png` all work. Each image must be the full in-game map for that
uiMapID, with nothing cropped (the game's maps are 3:2, for example 1002×668). The site
stretches it over the same 0–1 area the game's coordinates use, so the marker lines up.

Where to get them: export from your own game install with
[wow.export](https://github.com/Kruithne/wow.export), or take a screenshot of the in-game
world map. The art belongs to Blizzard, so keep the site non-commercial and keep the
copyright line in the footer. These files are git-ignored so they are not published with
the code.
