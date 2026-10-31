package liteparse

import "encoding/json"

// TextItem represents an extracted text item.
type TextItem struct {
	Text                   string    `json:"text"`
	X                      float64   `json:"x"`
	Y                      float64   `json:"y"`
	Width                  float64   `json:"width"`
	Height                 float64   `json:"height"`
	Rotation               float64   `json:"rotation"`
	FontName               *string   `json:"font_name"`
	FontSize               *float64  `json:"font_size"`
	Confidence             *float64  `json:"confidence"`
	FontHeight             *float64  `json:"font_height"`
	FontAscent             *float64  `json:"font_ascent"`
	FontDescent            *float64  `json:"font_descent"`
	FontWeight             *int      `json:"font_weight"`
	TextWidth              *float64  `json:"text_width"`
	FontIsBuggy            bool      `json:"font_is_buggy"`
	MCID                   *int      `json:"mcid"`
	FillColor              *string   `json:"fill_color"`
	StrokeColor            *string   `json:"stroke_color"`
	CharCodes              []uint32  `json:"char_codes"`
	TrailingSpaceGenerated bool      `json:"trailing_space_generated"`
	Strike                 bool      `json:"strike"`
	Link                   *string   `json:"link"`
	Words                  []WordBox `json:"words"`
}

// WordBox represents a word bounding box.
type WordBox struct {
	Text   string  `json:"text"`
	X      float64 `json:"x"`
	Y      float64 `json:"y"`
	Width  float64 `json:"width"`
	Height float64 `json:"height"`
}

// ParseBatch holds a batch parse result.
type ParseBatch struct {
	StartPage  int
	EndPage    int
	TotalPages int
	Result     ParseResult
}

// ScreenshotResult holds a rendered page screenshot.
type ScreenshotResult struct {
	PageNum    int    `json:"page_num"`
	Width      int    `json:"-"`
	Height     int    `json:"-"`
	ImageBytes []byte `json:"-"`
}

// ParsedPage represents a parsed page.
type ParsedPage struct {
	Page       int         `json:"-"`
	PageNum    int         `json:"page_num"`
	PageLabel  *string     `json:"page_label"`
	Width      float64     `json:"width"`
	Height     float64     `json:"height"`
	Text       string      `json:"text"`
	Markdown   string      `json:"markdown"`
	TextItems  []TextItem  `json:"text_items"`
	Complexity interface{} `json:"complexity"`
}

// Note: CLI JSON uses "page" not "page_num". Add custom unmarshal via alias? We'll handle with json tags page.
// We'll define separate CLI struct? Instead adjust tags.

// ParseResult represents parse output.
type ParseResult struct {
	TotalPages int          `json:"total_pages"`
	Pages      []ParsedPage `json:"pages"`
	Text       string       `json:"-"`
}

// LayoutComplexity holds layout signals.
type LayoutComplexity struct {
	ColumnCount        int      `json:"column_count"`
	RuledTableCount    int      `json:"ruled_table_count"`
	RuledTableCoverage float64  `json:"ruled_table_coverage"`
	TextTableRunCount  int      `json:"text_table_run_count"`
	FigureCount        int      `json:"figure_count"`
	FigureCoverage     float64  `json:"figure_coverage"`
	IsComplex          bool     `json:"is_complex"`
	Reasons            []string `json:"reasons"`
}

// PageComplexity holds per-page complexity signals.
type PageComplexity struct {
	PageNumber           int              `json:"page_number"`
	TextLength           int              `json:"text_length"`
	TextCoverage         float64          `json:"text_coverage"`
	HasSubstantialImages bool             `json:"has_substantial_images"`
	ImageBlockCount      int              `json:"image_block_count"`
	ImageCoverage        float64          `json:"image_coverage"`
	LargestImageCoverage float64          `json:"largest_image_coverage"`
	FullPageImage        bool             `json:"full_page_image"`
	UncoveredVectorArea  float64          `json:"uncovered_vector_area"`
	IsGarbled            bool             `json:"is_garbled"`
	PageArea             float64          `json:"page_area"`
	NeedsOCR             bool             `json:"needs_ocr"`
	Reasons              []string         `json:"reasons"`
	Layout               LayoutComplexity `json:"layout"`
}

// Ensure ParsedPage unmarshals "page" key.
func (p *ParsedPage) UnmarshalJSON(data []byte) error {
	type alias ParsedPage
	aux := struct {
		Page int `json:"page"`
		alias
	}{}
	if err := json.Unmarshal(data, &aux); err != nil {
		return err
	}
	p.PageNum = aux.Page
	p.Page = aux.Page
	p.PageLabel = aux.PageLabel
	p.Width = aux.Width
	p.Height = aux.Height
	p.Text = aux.Text
	p.Markdown = aux.Markdown
	p.TextItems = aux.TextItems
	p.Complexity = aux.Complexity
	return nil
}

func (p ParsedPage) MarshalJSON() ([]byte, error) {
	type alias ParsedPage
	return json.Marshal(struct {
		Page int `json:"page"`
		alias
	}{
		Page:  p.PageNum,
		alias: alias(p),
	})
}
