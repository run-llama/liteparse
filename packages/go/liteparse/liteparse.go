// Package liteparse provides Go support for LiteParse by wrapping the lit CLI.
package liteparse

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"image/png"
	"math"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
)

// Config holds LiteParse options. Fields map to CLI flags.
type Config struct {
	OCREnabled              *bool
	OCRLanguage             string
	OCRServerURL            string
	OCRServerHeaders        map[string]string
	TessdataPath            string
	MaxPages                int
	TargetPages             string
	ExtractScreenshots      bool
	ContinueOnPageError     bool
	DPI                     float64
	OutputFormat            string
	PreserveVerySmallText   *bool
	Password                string
	Quiet                   bool
	NumWorkers              int
	ImageMode               string
	ExtractImages           bool
	ImageOutputDir          string
	ExtractLinks            *bool
	KeepHeadersFooters      bool
	ExtractAnnotations      bool
	ExtractFormFields       bool
	ExtractStructureTree    bool
	ExtractBlocks           bool
	ExtractXFA              bool
	ExtractDocumentMetadata bool
	ExtractContentBounds    bool
	DetectScreenshotRects   bool
	ExtractVectorGraphics   bool
	ExtractTextMetadata     bool
	EmitWordBoxes           bool
	SkipDiagonalText        bool
	IncludeComplexity       bool
}

// Option configures a Client.
type Option func(*Config)

func WithOCREnabled(v bool) Option {
	return func(c *Config) { b := v; c.OCREnabled = &b }
}
func WithOCRLanguage(lang string) Option { return func(c *Config) { c.OCRLanguage = lang } }
func WithOCRServerURL(url string) Option { return func(c *Config) { c.OCRServerURL = url } }
func WithOCRServerHeaders(h map[string]string) Option {
	return func(c *Config) { c.OCRServerHeaders = h }
}
func WithTessdataPath(p string) Option { return func(c *Config) { c.TessdataPath = p } }
func WithMaxPages(n int) Option        { return func(c *Config) { c.MaxPages = n } }
func WithTargetPages(s string) Option  { return func(c *Config) { c.TargetPages = s } }
func WithPassword(pw string) Option    { return func(c *Config) { c.Password = pw } }
func WithQuiet(q bool) Option          { return func(c *Config) { c.Quiet = q } }
func WithOutputFormat(f string) Option { return func(c *Config) { c.OutputFormat = f } }
func WithDPI(d float64) Option         { return func(c *Config) { c.DPI = d } }
func WithNumWorkers(n int) Option      { return func(c *Config) { c.NumWorkers = n } }
func WithImageMode(m string) Option    { return func(c *Config) { c.ImageMode = m } }
func WithExtractImages(v bool) Option  { return func(c *Config) { c.ExtractImages = v } }
func WithImageOutputDir(d string) Option {
	return func(c *Config) { c.ImageOutputDir = d }
}
func WithExtractLinks(v bool) Option { return func(c *Config) { b := v; c.ExtractLinks = &b } }
func WithKeepHeadersFooters(v bool) Option {
	return func(c *Config) { c.KeepHeadersFooters = v }
}
func WithContinueOnPageError(v bool) Option {
	return func(c *Config) { c.ContinueOnPageError = v }
}
func WithExtractAnnotations(v bool) Option {
	return func(c *Config) { c.ExtractAnnotations = v }
}
func WithExtractFormFields(v bool) Option {
	return func(c *Config) { c.ExtractFormFields = v }
}
func WithExtractStructureTree(v bool) Option {
	return func(c *Config) { c.ExtractStructureTree = v }
}
func WithExtractBlocks(v bool) Option { return func(c *Config) { c.ExtractBlocks = v } }
func WithExtractXFA(v bool) Option    { return func(c *Config) { c.ExtractXFA = v } }
func WithExtractContentBounds(v bool) Option {
	return func(c *Config) { c.ExtractContentBounds = v }
}
func WithIncludeComplexity(v bool) Option {
	return func(c *Config) { c.IncludeComplexity = v }
}
func WithExtractTextMetadata(v bool) Option {
	return func(c *Config) { c.ExtractTextMetadata = v }
}
func WithExtractVectorGraphics(v bool) Option {
	return func(c *Config) { c.ExtractVectorGraphics = v }
}
func WithSkipDiagonalText(v bool) Option {
	return func(c *Config) { c.SkipDiagonalText = v }
}
func WithEmitWordBoxes(v bool) Option { return func(c *Config) { c.EmitWordBoxes = v } }
func WithPreserveVerySmallText(v bool) Option {
	return func(c *Config) { b := v; c.PreserveVerySmallText = &b }
}

// Client wraps the liteparse CLI.
type Client struct {
	cfg Config
	cli string
	mu  sync.Mutex
}

// LiteParse is an alias for Client.
type LiteParse = Client

// Parser is an alias for Client.
type Parser = Client

func New(opts ...Option) *Client {
	c := &Client{cfg: Config{MaxPages: 1000, OCRLanguage: "eng", OutputFormat: "json"}}
	for _, o := range opts {
		o(&c.cfg)
	}
	c.cli = resolveCLI()
	return c
}

// NewLiteParse is an alias for New.
func NewLiteParse(opts ...Option) *Client { return New(opts...) }

func resolveCLI() string {
	if v := os.Getenv("LITEPARSE_CLI_PATH"); v != "" {
		return v
	}
	if p, err := exec.LookPath("lit"); err == nil {
		return p
	}
	if p := buildFromSource(); p != "" {
		return p
	}
	for _, p := range []string{
		"./target/release/lit",
		"./target/debug/lit",
		"../target/release/lit",
		"../target/debug/lit",
		"../../target/release/lit",
		"../../target/debug/lit",
	} {
		if _, err := os.Stat(p); err == nil {
			return p
		}
	}
	return "lit"
}

var buildOnce sync.Once
var builtPath string

func buildFromSource() string {
	buildOnce.Do(func() {
		dir, _ := os.Getwd()
		for {
			if _, err := os.Stat(filepath.Join(dir, "Cargo.toml")); err == nil {
				b, _ := os.ReadFile(filepath.Join(dir, "Cargo.toml"))
				if strings.Contains(string(b), "members = [") {
					cmd := exec.Command("cargo", "build", "-p", "liteparse")
					cmd.Dir = dir
					cmd.Stdout = os.Stderr
					cmd.Stderr = os.Stderr
					if err := cmd.Run(); err == nil {
						for _, prof := range []string{"debug", "release"} {
							p := filepath.Join(dir, "target", prof, "lit")
							if _, err := os.Stat(p); err == nil {
								builtPath = p
								return
							}
						}
					}
					return
				}
			}
			parent := filepath.Dir(dir)
			if parent == dir {
				return
			}
			dir = parent
		}
	})
	return builtPath
}

// Parse accepts a file path (string) or raw bytes ([]byte).
func (c *Client) Parse(input interface{}) (ParseResult, error) {
	return c.ParseContext(context.Background(), input)
}

// ParseFile parses a document at the given path.
func (c *Client) ParseFile(path string) (ParseResult, error) {
	return c.ParseFileContext(context.Background(), path)
}

// ParseFileContext parses a document at the given path with context.
func (c *Client) ParseFileContext(ctx context.Context, path string) (ParseResult, error) {
	return c.runParse(ctx, path)
}

// ParseContext parses input with context.
func (c *Client) ParseContext(ctx context.Context, input interface{}) (ParseResult, error) {
	switch v := input.(type) {
	case string:
		return c.runParse(ctx, v)
	case []byte:
		tmp, err := os.CreateTemp("", "liteparse-*.pdf")
		if err != nil {
			return ParseResult{}, err
		}
		defer os.Remove(tmp.Name())
		if _, err := tmp.Write(v); err != nil {
			tmp.Close()
			return ParseResult{}, err
		}
		tmp.Close()
		return c.runParse(ctx, tmp.Name())
	default:
		return ParseResult{}, fmt.Errorf("unsupported input type %T", input)
	}
}

// ParseBytes parses raw PDF bytes written to a temp file.
func (c *Client) ParseBytes(data []byte) (ParseResult, error) {
	tmp, err := os.CreateTemp("", "liteparse-*.pdf")
	if err != nil {
		return ParseResult{}, err
	}
	defer os.Remove(tmp.Name())
	if _, err := tmp.Write(data); err != nil {
		tmp.Close()
		return ParseResult{}, err
	}
	tmp.Close()
	return c.runParse(context.Background(), tmp.Name())
}

// ParseInput is an alias.
func (c *Client) ParseInput(input interface{}) (ParseResult, error) { return c.Parse(input) }
func (c *Client) ParsePath(path string) (ParseResult, error)        { return c.ParseFile(path) }

func (c *Client) runParse(ctx context.Context, file string) (ParseResult, error) {
	c.mu.Lock()
	defer c.mu.Unlock()

	args := []string{"parse", file, "--format", "json"}
	if c.cfg.Quiet {
		args = append(args, "--quiet")
	}
	if c.cfg.MaxPages > 0 {
		args = append(args, "--max-pages", fmt.Sprintf("%d", c.cfg.MaxPages))
	}
	if c.cfg.TargetPages != "" {
		args = append(args, "--target-pages", c.cfg.TargetPages)
	}
	if c.cfg.Password != "" {
		args = append(args, "--password", c.cfg.Password)
	}
	if c.cfg.OCRLanguage != "" {
		args = append(args, "--ocr-language", c.cfg.OCRLanguage)
	}
	if ptrBool(c.cfg.OCREnabled, true) == false {
		args = append(args, "--no-ocr")
	}
	if c.cfg.OCRServerURL != "" {
		args = append(args, "--ocr-server-url", c.cfg.OCRServerURL)
	}
	for k, v := range c.cfg.OCRServerHeaders {
		args = append(args, "--ocr-server-header", k+": "+v)
	}
	if c.cfg.TessdataPath != "" {
		args = append(args, "--tessdata-path", c.cfg.TessdataPath)
	}
	if c.cfg.DPI > 0 {
		args = append(args, "--dpi", fmt.Sprintf("%.0f", c.cfg.DPI))
	}
	if ptrBool(c.cfg.PreserveVerySmallText, false) {
		args = append(args, "--preserve-small-text")
	}
	if c.cfg.NumWorkers > 0 {
		args = append(args, "--num-workers", fmt.Sprintf("%d", c.cfg.NumWorkers))
	}
	if c.cfg.ImageMode != "" {
		args = append(args, "--image-mode", c.cfg.ImageMode)
	}
	if c.cfg.ExtractImages {
		args = append(args, "--extract-images")
	}
	if c.cfg.ImageOutputDir != "" {
		args = append(args, "--image-output-dir", c.cfg.ImageOutputDir)
	}
	if ptrBool(c.cfg.ExtractLinks, true) == false {
		args = append(args, "--no-links")
	}
	if c.cfg.KeepHeadersFooters {
		args = append(args, "--keep-headers-footers")
	}
	if c.cfg.ContinueOnPageError {
		args = append(args, "--continue-on-page-error")
	}
	if c.cfg.ExtractAnnotations {
		args = append(args, "--extract-annotations")
	}
	if c.cfg.ExtractFormFields {
		args = append(args, "--extract-form-fields")
	}
	if c.cfg.ExtractStructureTree {
		args = append(args, "--extract-structure-tree")
	}
	if c.cfg.ExtractBlocks {
		args = append(args, "--extract-blocks")
	}
	if c.cfg.ExtractXFA {
		args = append(args, "--extract-xfa-packets")
	}
	if c.cfg.ExtractContentBounds {
		args = append(args, "--extract-content-bounds")
	}
	if c.cfg.IncludeComplexity {
		args = append(args, "--complexity")
	}
	if c.cfg.ExtractTextMetadata {
		args = append(args, "--extract-text-metadata")
	}
	if c.cfg.ExtractVectorGraphics {
		args = append(args, "--extract-vector-graphics")
	}
	if c.cfg.SkipDiagonalText {
		args = append(args, "--skip-diagonal-text")
	}
	if c.cfg.EmitWordBoxes {
		args = append(args, "--emit-word-boxes")
	}

	cmd := exec.CommandContext(ctx, c.cli, args...)
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		return ParseResult{}, fmt.Errorf("liteparse: %w: %s", err, stderr.String())
	}

	raw := stdout.Bytes()
	idx := bytes.IndexByte(raw, '{')
	if idx < 0 {
		return ParseResult{}, fmt.Errorf("no JSON output: %s", string(raw))
	}
	raw = raw[idx:]

	var cli struct {
		TotalPages int `json:"total_pages"`
		Pages      []struct {
			Page       int        `json:"page"`
			PageLabel  *string    `json:"page_label"`
			Width      float64    `json:"width"`
			Height     float64    `json:"height"`
			Text       string     `json:"text"`
			Markdown   string     `json:"markdown"`
			TextItems  []TextItem `json:"text_items"`
			Complexity *any       `json:"complexity"`
		} `json:"pages"`
	}
	if err := json.Unmarshal(raw, &cli); err != nil {
		return ParseResult{}, fmt.Errorf("parse json: %w", err)
	}

	res := ParseResult{TotalPages: cli.TotalPages}
	var textBuf strings.Builder
	for _, p := range cli.Pages {
		page := ParsedPage{
			Page:       p.Page,
			PageNum:    p.Page,
			PageLabel:  p.PageLabel,
			Width:      p.Width,
			Height:     p.Height,
			Text:       p.Text,
			Markdown:   p.Markdown,
			TextItems:  p.TextItems,
			Complexity: p.Complexity,
		}
		res.Pages = append(res.Pages, page)
		textBuf.WriteString(p.Text)
		textBuf.WriteString("\n")
	}
	res.Text = textBuf.String()
	if res.TotalPages == 0 {
		res.TotalPages = len(res.Pages)
	}
	return res, nil
}

func (c *Config) NoOCR() bool {
	if c.OCREnabled != nil {
		return !*c.OCREnabled
	}
	return false
}

// IsComplex runs the is-complex command.
func (c *Client) IsComplex(input interface{}) ([]PageComplexity, error) {
	path, cleanup, err := toPath(input)
	if err != nil {
		return nil, err
	}
	defer cleanup()

	args := []string{"is-complex", path}
	if c.cfg.MaxPages > 0 {
		args = append(args, "--max-pages", fmt.Sprintf("%d", c.cfg.MaxPages))
	}
	if c.cfg.TargetPages != "" {
		args = append(args, "--target-pages", c.cfg.TargetPages)
	}
	if c.cfg.Password != "" {
		args = append(args, "--password", c.cfg.Password)
	}
	if c.cfg.Quiet {
		args = append(args, "--quiet")
	}
	cmd := exec.Command(c.cli, args...)
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		return nil, fmt.Errorf("liteparse: %w: %s", err, stderr.String())
	}
	raw := stdout.Bytes()
	idx := bytes.IndexByte(raw, '[')
	if idx < 0 {
		return nil, nil
	}
	raw = raw[idx:]
	end := bytes.LastIndexByte(raw, ']')
	if end >= 0 {
		raw = raw[:end+1]
	}
	var res []PageComplexity
	if err := json.Unmarshal(raw, &res); err != nil {
		return nil, fmt.Errorf("parse json: %w", err)
	}
	return res, nil
}

// Screenshot renders pages to PNG and returns their bytes.
func (c *Client) Screenshot(input interface{}, pageNumbers []int) ([]ScreenshotResult, error) {
	path, cleanup, err := toPath(input)
	if err != nil {
		return nil, err
	}
	defer cleanup()

	outDir, err := os.MkdirTemp("", "liteparse-shots-*")
	if err != nil {
		return nil, err
	}
	defer os.RemoveAll(outDir)

	args := []string{"screenshot", path, "--output-dir", outDir}
	if len(pageNumbers) > 0 {
		parts := make([]string, len(pageNumbers))
		for i, n := range pageNumbers {
			parts[i] = fmt.Sprintf("%d", n)
		}
		args = append(args, "--target-pages", strings.Join(parts, ","))
	}
	if c.cfg.Password != "" {
		args = append(args, "--password", c.cfg.Password)
	}
	if c.cfg.DPI > 0 {
		args = append(args, "--dpi", fmt.Sprintf("%.0f", c.cfg.DPI))
	}
	if c.cfg.Quiet {
		args = append(args, "--quiet")
	}

	cmd := exec.Command(c.cli, args...)
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	if err := cmd.Run(); err != nil {
		return nil, fmt.Errorf("liteparse: %w: %s", err, stderr.String())
	}

	entries, err := os.ReadDir(outDir)
	if err != nil {
		return nil, err
	}
	var results []ScreenshotResult
	for _, e := range entries {
		if e.IsDir() {
			continue
		}
		name := e.Name()
		if !strings.HasPrefix(name, "page_") || !strings.HasSuffix(name, ".png") {
			continue
		}
		numStr := strings.TrimPrefix(name, "page_")
		numStr = strings.TrimSuffix(numStr, ".png")
		n, _ := strconv.Atoi(numStr)
		data, err := os.ReadFile(filepath.Join(outDir, name))
		if err != nil {
			continue
		}
		w, h := 0, 0
		if cfg, err := png.DecodeConfig(bytes.NewReader(data)); err == nil {
			w, h = cfg.Width, cfg.Height
		}
		results = append(results, ScreenshotResult{
			PageNum:    n,
			Width:      w,
			Height:     h,
			ImageBytes: data,
		})
	}
	return results, nil
}

func toPath(input interface{}) (string, func(), error) {
	switch v := input.(type) {
	case string:
		return v, func() {}, nil
	case []byte:
		tmp, err := os.CreateTemp("", "liteparse-*.pdf")
		if err != nil {
			return "", nil, err
		}
		if _, err := tmp.Write(v); err != nil {
			tmp.Close()
			os.Remove(tmp.Name())
			return "", nil, err
		}
		tmp.Close()
		return tmp.Name(), func() { os.Remove(tmp.Name()) }, nil
	default:
		return "", nil, fmt.Errorf("unsupported input type %T", v)
	}
}

// Package-level helpers
func Parse(input interface{}) (ParseResult, error) { return New().Parse(input) }
func ParseFile(path string) (ParseResult, error)   { return New().ParseFile(path) }
func ParseBytes(data []byte) (ParseResult, error)  { return New().ParseBytes(data) }
func ParseContext(ctx context.Context, input interface{}) (ParseResult, error) {
	return New().ParseContext(ctx, input)
}
func ParseFileContext(ctx context.Context, path string) (ParseResult, error) {
	return New().ParseFileContext(ctx, path)
}
func IsComplex(input interface{}) ([]PageComplexity, error) {
	return New().IsComplex(input)
}
func Screenshot(input interface{}, pageNumbers []int) ([]ScreenshotResult, error) {
	return New().Screenshot(input, pageNumbers)
}

// SearchItems searches text items for phrase matches.
func SearchItems(items []TextItem, phrase string, caseSensitive bool) []TextItem {
	var results []TextItem
	if phrase == "" {
		return results
	}
	normalize := func(s string) string {
		if caseSensitive {
			return s
		}
		return strings.ToLower(s)
	}
	q := normalize(phrase)

	seps := make([]string, len(items))
	for i := 1; i < len(items); i++ {
		prev := items[i-1]
		cur := items[i]
		fontSize := 12.0
		if prev.FontSize != nil {
			fontSize = *prev.FontSize
		} else if cur.FontSize != nil {
			fontSize = *cur.FontSize
		}
		sameLine := math.Abs(cur.Y-prev.Y) < fontSize*0.5
		gap := cur.X - (prev.X + prev.Width)
		if sameLine && gap <= fontSize*0.3 {
			seps[i] = ""
		} else {
			seps[i] = " "
		}
	}

	start := 0
	for start < len(items) {
		combined := ""
		found := false
		for end := start; end < len(items); end++ {
			if end > start {
				combined += seps[end]
			}
			combined += items[end].Text

			if strings.Contains(normalize(combined), q) {
				narrowed := combined
				s := start
				for s < end {
					skipLen := len(items[s].Text) + len(seps[s+1])
					without := narrowed[skipLen:]
					if strings.Contains(normalize(without), q) {
						narrowed = without
						s++
					} else {
						break
					}
				}

				matched := items[s : end+1]
				x := matched[0].X
				y := matched[0].Y
				x2 := matched[0].X + matched[0].Width
				y2 := matched[0].Y + matched[0].Height
				for _, m := range matched[1:] {
					if m.X < x {
						x = m.X
					}
					if m.Y < y {
						y = m.Y
					}
					if m.X+m.Width > x2 {
						x2 = m.X + m.Width
					}
					if m.Y+m.Height > y2 {
						y2 = m.Y + m.Height
					}
				}

				res := TextItem{
					Text:     phrase,
					X:        x,
					Y:        y,
					Width:    x2 - x,
					Height:   y2 - y,
					FontName: matched[0].FontName,
					FontSize: matched[0].FontSize,
				}
				results = append(results, res)

				start = end + 1
				found = true
				break
			}

			if len(combined) > len(q)*2 {
				break
			}
		}
		if !found {
			start++
		}
	}
	return results
}

// ParseBatches parses a document in page batches.
func (c *Client) ParseBatches(input interface{}, batchSize int) ([]ParseBatch, error) {
	if batchSize <= 0 {
		batchSize = 25
	}
	full, err := c.Parse(input)
	if err != nil {
		return nil, err
	}
	total := full.TotalPages
	if total == 0 {
		total = len(full.Pages)
	}
	// collect page numbers
	pages := make([]int, 0, total)
	for i := 1; i <= total; i++ {
		pages = append(pages, i)
	}
	var batches []ParseBatch
	for start := 0; start < len(pages); start += batchSize {
		end := start + batchSize
		if end > len(pages) {
			end = len(pages)
		}
		chunk := pages[start:end]
		parts := make([]string, len(chunk))
		for i, p := range chunk {
			parts[i] = fmt.Sprintf("%d", p)
		}
		// clone config with target pages
		cfg := c.cfg
		cfg.TargetPages = strings.Join(parts, ",")
		client := &Client{cfg: cfg, cli: c.cli}
		res, err := client.Parse(input)
		if err != nil {
			return nil, err
		}
		batches = append(batches, ParseBatch{
			StartPage:  chunk[0],
			EndPage:    chunk[len(chunk)-1],
			TotalPages: total,
			Result:     res,
		})
	}
	return batches, nil
}

func ptrBool(p *bool, def bool) bool {
	if p == nil {
		return def
	}
	return *p
}

func boolPtr(p *bool) bool {
	return p != nil && *p
}
func boolPtrFalse(p *bool) bool {
	return p != nil && !*p
}
