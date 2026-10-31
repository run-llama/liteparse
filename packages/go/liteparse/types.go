package liteparse

import "encoding/json"

// TextItem represents an extracted text item.
type TextItem struct {
	Text                   string    `json:"-"`
	X                      float64   `json:"-"`
	Y                      float64   `json:"-"`
	Width                  float64   `json:"-"`
	Height                 float64   `json:"-"`
	Rotation               float64   `json:"-"`
	FontName               *string   `json:"-"`
	FontSize               *float64  `json:"-"`
	Confidence             *float64  `json:"-"`
	FontHeight             *float64  `json:"-"`
	FontAscent             *float64  `json:"-"`
	FontDescent            *float64  `json:"-"`
	FontWeight             *int      `json:"-"`
	TextWidth              *float64  `json:"-"`
	FontIsBuggy            bool      `json:"-"`
	MCID                   *int      `json:"-"`
	FillColor              *string   `json:"-"`
	StrokeColor            *string   `json:"-"`
	CharCodes              []uint32  `json:"-"`
	TrailingSpaceGenerated bool      `json:"-"`
	Strike                 bool      `json:"-"`
	Link                   *string   `json:"-"`
	Words                  []WordBox `json:"-"`
	// exported JSON fields are handled via custom marshal/unmarshal
}

// We keep JSON tags via a parallel raw struct.
type textItemRaw struct {
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

	// camelCase (Node CLI)
	TextItemsCamel     string   `json:"-"`
	FontNameCamel      *string  `json:"fontName"`
	FontSizeCamel      *float64 `json:"fontSize"`
	FontHeightCamel    *float64 `json:"fontHeight"`
	FontWeightCamel    *int     `json:"fontWeight"`
	TextWidthCamel     *float64 `json:"textWidth"`
	MCIDCamel          *int     `json:"mcid"`
	FillColorCamel     *string  `json:"fillColor"`
	StrokeColorCamel   *string  `json:"strokeColor"`
	CharCodesCamel     []uint32 `json:"charCodes"`
	TrailingSpaceCamel bool     `json:"trailingSpaceGenerated"`
}

func (t *TextItem) UnmarshalJSON(data []byte) error {
	var raw textItemRaw
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}
	t.Text = raw.Text
	t.X = raw.X
	t.Y = raw.Y
	t.Width = raw.Width
	t.Height = raw.Height
	t.Rotation = raw.Rotation
	t.FontName = raw.FontName
	if t.FontName == nil {
		t.FontName = raw.FontNameCamel
	}
	t.FontSize = raw.FontSize
	if t.FontSize == nil {
		t.FontSize = raw.FontSizeCamel
	}
	t.Confidence = raw.Confidence
	t.FontHeight = raw.FontHeight
	if t.FontHeight == nil {
		t.FontHeight = raw.FontHeightCamel
	}
	t.FontAscent = raw.FontAscent
	t.FontDescent = raw.FontDescent
	t.FontWeight = raw.FontWeight
	if t.FontWeight == nil {
		t.FontWeight = raw.FontWeightCamel
	}
	t.TextWidth = raw.TextWidth
	if t.TextWidth == nil {
		t.TextWidth = raw.TextWidthCamel
	}
	t.FontIsBuggy = raw.FontIsBuggy
	t.MCID = raw.MCID
	if t.MCID == nil {
		t.MCID = raw.MCIDCamel
	}
	t.FillColor = raw.FillColor
	if t.FillColor == nil {
		t.FillColor = raw.FillColorCamel
	}
	t.StrokeColor = raw.StrokeColor
	if t.StrokeColor == nil {
		t.StrokeColor = raw.StrokeColorCamel
	}
	t.CharCodes = raw.CharCodes
	if t.CharCodes == nil {
		t.CharCodes = raw.CharCodesCamel
	}
	t.TrailingSpaceGenerated = raw.TrailingSpaceGenerated || raw.TrailingSpaceCamel
	t.Strike = raw.Strike
	t.Link = raw.Link
	t.Words = raw.Words
	return nil
}

func (t TextItem) MarshalJSON() ([]byte, error) {
	raw := textItemRaw{
		Text:                   t.Text,
		X:                      t.X,
		Y:                      t.Y,
		Width:                  t.Width,
		Height:                 t.Height,
		Rotation:               t.Rotation,
		FontName:               t.FontName,
		FontSize:               t.FontSize,
		Confidence:             t.Confidence,
		FontHeight:             t.FontHeight,
		FontAscent:             t.FontAscent,
		FontDescent:            t.FontDescent,
		FontWeight:             t.FontWeight,
		TextWidth:              t.TextWidth,
		FontIsBuggy:            t.FontIsBuggy,
		MCID:                   t.MCID,
		FillColor:              t.FillColor,
		StrokeColor:            t.StrokeColor,
		CharCodes:              t.CharCodes,
		TrailingSpaceGenerated: t.TrailingSpaceGenerated,
		Strike:                 t.Strike,
		Link:                   t.Link,
		Words:                  t.Words,
	}
	return json.Marshal(raw)
}

// WordBox represents a word bounding box.
type WordBox struct {
	Text   string  `json:"text"`
	X      float64 `json:"x"`
	Y      float64 `json:"y"`
	Width  float64 `json:"width"`
	Height float64 `json:"height"`
}

// ParsedPage represents a parsed page.
type ParsedPage struct {
	Page       int         `json:"-"`
	PageNum    int         `json:"-"`
	PageLabel  *string     `json:"-"`
	Width      float64     `json:"-"`
	Height     float64     `json:"-"`
	Text       string      `json:"-"`
	Markdown   string      `json:"-"`
	TextItems  []TextItem  `json:"-"`
	Complexity interface{} `json:"-"`
}

type parsedPageRaw struct {
	Page       int         `json:"page"`
	PageNum    int         `json:"page_num"`
	PageNumber int         `json:"page_number"`
	PageLabel  *string     `json:"page_label"`
	Width      float64     `json:"width"`
	Height     float64     `json:"height"`
	Text       string      `json:"text"`
	Markdown   string      `json:"markdown"`
	TextItems  []TextItem  `json:"text_items"`
	TextItemsC []TextItem  `json:"textItems"`
	Complexity interface{} `json:"complexity"`
}

func (p *ParsedPage) UnmarshalJSON(data []byte) error {
	var raw parsedPageRaw
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}
	if raw.Page != 0 {
		p.Page = raw.Page
	} else if raw.PageNum != 0 {
		p.Page = raw.PageNum
	} else {
		p.Page = raw.PageNumber
	}
	p.PageNum = p.Page
	p.PageLabel = raw.PageLabel
	p.Width = raw.Width
	p.Height = raw.Height
	p.Text = raw.Text
	p.Markdown = raw.Markdown
	if len(raw.TextItems) > 0 {
		p.TextItems = raw.TextItems
	} else {
		p.TextItems = raw.TextItemsC
	}
	p.Complexity = raw.Complexity
	return nil
}

func (p ParsedPage) MarshalJSON() ([]byte, error) {
	raw := parsedPageRaw{
		Page:       p.Page,
		PageLabel:  p.PageLabel,
		Width:      p.Width,
		Height:     p.Height,
		Text:       p.Text,
		Markdown:   p.Markdown,
		TextItems:  p.TextItems,
		Complexity: p.Complexity,
	}
	return json.Marshal(raw)
}

// ParseResult represents parse output.
type ParseResult struct {
	TotalPages int          `json:"-"`
	Pages      []ParsedPage `json:"-"`
	Text       string       `json:"-"`
}

type parseResultRaw struct {
	TotalPages  int          `json:"total_pages"`
	TotalPages2 int          `json:"totalPages"`
	Pages       []ParsedPage `json:"pages"`
}

func (r *ParseResult) UnmarshalJSON(data []byte) error {
	var raw parseResultRaw
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}
	if raw.TotalPages != 0 {
		r.TotalPages = raw.TotalPages
	} else {
		r.TotalPages = raw.TotalPages2
	}
	r.Pages = raw.Pages
	if r.TotalPages == 0 {
		r.TotalPages = len(r.Pages)
	}
	var b string
	for _, p := range r.Pages {
		b += p.Text + "\n"
	}
	r.Text = b
	return nil
}

func (r ParseResult) MarshalJSON() ([]byte, error) {
	return json.Marshal(struct {
		TotalPages int          `json:"total_pages"`
		Pages      []ParsedPage `json:"pages"`
	}{
		TotalPages: r.TotalPages,
		Pages:      r.Pages,
	})
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

// ScreenshotResult holds a rendered page screenshot.
type ScreenshotResult struct {
	PageNum    int    `json:"page_num"`
	Width      int    `json:"-"`
	Height     int    `json:"-"`
	ImageBytes []byte `json:"-"`
}

// ParseBatch holds a batch parse result.
type ParseBatch struct {
	StartPage  int
	EndPage    int
	TotalPages int
	Result     ParseResult
}
