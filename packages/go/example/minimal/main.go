package main

import (
	"fmt"
	"log"
	"time"

	liteparse "github.com/run-llama/liteparse/packages/go"
)

func main() {
	// 测试解析 DataDictionary PDF
	pdfPath := "/Users/xxdld/Documents/DataDictionary_20260122153143.pdf"

	parser := liteparse.New("")

	// 配置解析选项
	opts := liteparse.DefaultParseOptions()
	opts.OutputFormat = liteparse.OutputFormatJSON
	opts.OCREnabled = liteparse.Bool(false)
	opts.Timeout = 30 * time.Second

	fmt.Printf("开始解析 PDF: %s\n", pdfPath)
	start := time.Now()

	result, err := parser.Parse(pdfPath, &opts)
	if err != nil {
		log.Fatalf("解析失败: %v", err)
	}

	duration := time.Since(start)

	// 打印解析结果
	fmt.Printf("\n========== 解析结果 ==========\n")
	fmt.Printf("耗时: %v\n", duration)
	fmt.Printf("页数: %d\n", result.NumPages())
	fmt.Printf("输出格式: %s\n", result.OutputFormat)
	fmt.Printf("文本长度: %d 字符\n\n", len(result.Text))

	// 打印前几页的详细信息
	fmt.Printf("========== 页面详情 ==========\n")
	maxPages := 5
	if result.NumPages() < maxPages {
		maxPages = result.NumPages()
	}

	for i := 1; i <= maxPages; i++ {
		page := result.GetPage(i)
		if page != nil {
			fmt.Printf("第 %d 页: %d 个文本项, %d 字符\n",
				i, len(page.TextItems), len(page.Text))
		}
	}

	// 打印文本预览（前 500 字符）
	fmt.Printf("\n========== 文本预览 ==========\n")
	preview := result.Text
	if len(preview) > 500 {
		preview = preview[:500] + "..."
	}
	fmt.Println(preview)
}
