package liteparse

import "time"

// OutputFormat is the output format used by LiteParse CLI commands.
type OutputFormat string

const (
	OutputFormatJSON OutputFormat = "json"
	OutputFormatText OutputFormat = "text"
)

// ImageFormat is the image format used for screenshot generation.
type ImageFormat string

const (
	ImageFormatPNG ImageFormat = "png"
	ImageFormatJPG ImageFormat = "jpg"
)

// BoundingBox contains page coordinates for a text span.
type BoundingBox struct {
	X1 float64 `json:"x1"`
	Y1 float64 `json:"y1"`
	X2 float64 `json:"x2"`
	Y2 float64 `json:"y2"`
}

// TextItem is an individual text item extracted from a page.
type TextItem struct {
	Str      string   `json:"str"`
	X        float64  `json:"x"`
	Y        float64  `json:"y"`
	Width    float64  `json:"width"`
	Height   float64  `json:"height"`
	W        float64  `json:"w"`
	H        float64  `json:"h"`
	R        float64  `json:"r"`
	FontName *string  `json:"fontName,omitempty"`
	FontSize *float64 `json:"fontSize,omitempty"`
}

// ParsedPage contains the extracted content for a single page.
type ParsedPage struct {
	PageNum       int           `json:"pageNum"`
	Width         float64       `json:"width"`
	Height        float64       `json:"height"`
	Text          string        `json:"text"`
	TextItems     []TextItem    `json:"textItems"`
	BoundingBoxes []BoundingBox `json:"boundingBoxes"`
}

// ParseResult is the structured result of parsing a single document.
type ParseResult struct {
	OutputFormat OutputFormat   `json:"outputFormat"`
	Pages        []ParsedPage   `json:"pages"`
	Text         string         `json:"text"`
	JSON         map[string]any `json:"json,omitempty"`
}

// NumPages returns the number of parsed pages.
func (r *ParseResult) NumPages() int {
	if r == nil {
		return 0
	}
	return len(r.Pages)
}

// GetPage returns the page with the matching 1-based page number.
func (r *ParseResult) GetPage(pageNum int) *ParsedPage {
	if r == nil {
		return nil
	}
	for i := range r.Pages {
		if r.Pages[i].PageNum == pageNum {
			return &r.Pages[i]
		}
	}
	return nil
}

// BatchResult describes a completed batch parse run.
type BatchResult struct {
	OutputDir string `json:"outputDir"`
}

// ScreenshotResult describes a rendered page image.
type ScreenshotResult struct {
	PageNum    int    `json:"pageNum"`
	ImagePath  string `json:"imagePath"`
	ImageBytes []byte `json:"imageBytes,omitempty"`
}

// ScreenshotBatchResult contains all screenshots emitted by a run.
type ScreenshotBatchResult struct {
	Screenshots []ScreenshotResult `json:"screenshots"`
	OutputDir   string             `json:"outputDir"`
}

// Len returns the number of screenshots in the batch.
func (r *ScreenshotBatchResult) Len() int {
	if r == nil {
		return 0
	}
	return len(r.Screenshots)
}

// GetPage returns the screenshot for the matching 1-based page number.
func (r *ScreenshotBatchResult) GetPage(pageNum int) *ScreenshotResult {
	if r == nil {
		return nil
	}
	for i := range r.Screenshots {
		if r.Screenshots[i].PageNum == pageNum {
			return &r.Screenshots[i]
		}
	}
	return nil
}

// ParseOptions configures the Parse command.
type ParseOptions struct {
	OutputFormat          OutputFormat
	OCREnabled            *bool
	OCRServerURL          string
	OCRLanguage           string
	MaxPages              int
	TargetPages           string
	DPI                   int
	PreciseBoundingBox    *bool
	SkipDiagonalText      *bool
	PreserveVerySmallText *bool
	Timeout               time.Duration
}

// DefaultParseOptions returns ParseOptions with the same defaults as the Python package.
func DefaultParseOptions() ParseOptions {
	return ParseOptions{
		OutputFormat:       OutputFormatText,
		OCREnabled:         Bool(true),
		OCRLanguage:        "en",
		MaxPages:           1000,
		DPI:                150,
		PreciseBoundingBox: Bool(true),
	}
}

// BatchParseOptions configures the BatchParse command.
type BatchParseOptions struct {
	OutputFormat       OutputFormat
	OCREnabled         *bool
	OCRServerURL       string
	OCRLanguage        string
	MaxPages           int
	DPI                int
	PreciseBoundingBox *bool
	Recursive          *bool
	ExtensionFilter    string
	Timeout            time.Duration
}

// DefaultBatchParseOptions returns BatchParseOptions with the same defaults as the Python package.
func DefaultBatchParseOptions() BatchParseOptions {
	return BatchParseOptions{
		OutputFormat:       OutputFormatText,
		OCREnabled:         Bool(true),
		OCRLanguage:        "en",
		MaxPages:           1000,
		DPI:                150,
		PreciseBoundingBox: Bool(true),
		Recursive:          Bool(false),
	}
}

// ScreenshotOptions configures the Screenshot command.
type ScreenshotOptions struct {
	TargetPages string
	DPI         int
	ImageFormat ImageFormat
	LoadBytes   *bool
	Timeout     time.Duration
}

// DefaultScreenshotOptions returns ScreenshotOptions with the same defaults as the Python package.
func DefaultScreenshotOptions() ScreenshotOptions {
	return ScreenshotOptions{
		DPI:         150,
		ImageFormat: ImageFormatPNG,
		LoadBytes:   Bool(false),
	}
}

// Bool returns a pointer to a bool value for option structs.
func Bool(v bool) *bool {
	return &v
}
