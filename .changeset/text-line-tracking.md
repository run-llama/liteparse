---
"@llamaindex/liteparse": minor
---

Add text line coordinate tracking and legal line number detection. When `textLineTracking` is enabled, each parsed page includes a `textLines` array where every entry carries the reconstructed line text, its union PDF bounding box, and an optional detected printed line number. Also adds `searchTextLines()` utility for finding text across lines and retrieving their bounding boxes.
