package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"log"
	"os"
	"time"

	liteparse "github.com/run-llama/liteparse/packages/go"
)

func main() {
	// 命令行参数
	pdfPath := flag.String("pdf", "/Users/xxdld/Documents/DataDictionary_20260122153143.pdf", "PDF 文件路径")
	outputFormat := flag.String("format", "json", "输出格式: json 或 text")
	enableOCR := flag.Bool("ocr", false, "是否启用 OCR")
	targetPages := flag.String("pages", "", "目标页面，例如: 1-3 或 1,3,5")
	saveJSON := flag.String("save", "", "保存 JSON 结果到文件")
	flag.Parse()

	parser := liteparse.New("")

	// 配置解析选项
	opts := liteparse.DefaultParseOptions()

	if *outputFormat == "text" {
		opts.OutputFormat = liteparse.OutputFormatText
	} else {
		opts.OutputFormat = liteparse.OutputFormatJSON
	}

	opts.OCREnabled = liteparse.Bool(*enableOCR)
	opts.Timeout = 60 * time.Second

	if *targetPages != "" {
		opts.TargetPages = *targetPages
	}

	fmt.Printf("========== 配置信息 ==========\n")
	fmt.Printf("PDF 文件: %s\n", *pdfPath)
	fmt.Printf("输出格式: %s\n", *outputFormat)
	fmt.Printf("启用 OCR: %v\n", *enableOCR)
	if *targetPages != "" {
		fmt.Printf("目标页面: %s\n", *targetPages)
	}
	fmt.Println()

	fmt.Printf("开始解析...\n")
	start := time.Now()

	result, err := parser.Parse(*pdfPath, &opts)
	if err != nil {
		log.Fatalf("解析失败: %v", err)
	}

	duration := time.Since(start)

	// 打印解析结果
	fmt.Printf("\n========== 解析结果 ==========\n")
	fmt.Printf("耗时: %v\n", duration)
	fmt.Printf("页数: %d\n", result.NumPages())
	fmt.Printf("输出格式: %s\n", result.OutputFormat)
	fmt.Printf("文本长度: %d 字符\n", len(result.Text))

	// 如果是 JSON 格式，打印页面详情
	if result.OutputFormat == liteparse.OutputFormatJSON && result.NumPages() > 0 {
		fmt.Printf("\n========== 页面详情 ==========\n")
		maxPages := 10
		if result.NumPages() < maxPages {
			maxPages = result.NumPages()
		}

		for i := 1; i <= maxPages; i++ {
			page := result.GetPage(i)
			if page != nil {
				fmt.Printf("第 %d 页: %d 个文本项, %d 字符, 尺寸: %.0fx%.0f\n",
					i, len(page.TextItems), len(page.Text), page.Width, page.Height)
			}
		}

		if result.NumPages() > maxPages {
			fmt.Printf("... 还有 %d 页\n", result.NumPages()-maxPages)
		}
	}

	// 打印文本预览
	fmt.Printf("\n========== 文本预览 (前 1000 字符) ==========\n")
	preview := result.Text
	if len(preview) > 1000 {
		preview = preview[:1000] + "\n... (省略 " + fmt.Sprintf("%d", len(result.Text)-1000) + " 字符)"
	}
	fmt.Println(preview)

	// 保存 JSON 结果到文件
	if *saveJSON != "" && result.JSON != nil {
		fmt.Printf("\n========== 保存结果 ==========\n")
		jsonData, err := json.MarshalIndent(result.JSON, "", "  ")
		if err != nil {
			log.Printf("JSON 序列化失败: %v", err)
		} else {
			if err := os.WriteFile(*saveJSON, jsonData, 0644); err != nil {
				log.Printf("保存文件失败: %v", err)
			} else {
				fmt.Printf("JSON 结果已保存到: %s\n", *saveJSON)
				fmt.Printf("文件大小: %d 字节\n", len(jsonData))
			}
		}
	}

	fmt.Printf("\n解析完成！\n")
}
