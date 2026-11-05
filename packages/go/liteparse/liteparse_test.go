package liteparse

import (
	"os"
	"testing"
)

func TestParseFile(t *testing.T) {
	p := "../../../integration_tests_data/sample.pdf"
	// adjust path from package dir
	if _, err := os.Stat(p); err != nil {
		t.Skip("sample pdf not found")
	}
	res, err := ParseFile(p)
	if err != nil {
		t.Fatal(err)
	}
	if res.TotalPages != 1 {
		t.Fatalf("total pages %d", res.TotalPages)
	}
	if len(res.Pages) == 0 || res.Pages[0].Text == "" {
		t.Fatal("no text")
	}
	t.Logf("chars: %d", len(res.Text))

	// IsComplex
	stats, err := IsComplex("../../../integration_tests_data/sample.pdf")
	if err != nil {
		t.Fatal(err)
	}
	if len(stats) == 0 {
		t.Fatal("no complexity stats")
	}

	// Screenshot
	shots, err := Screenshot("../../../integration_tests_data/sample.pdf", nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(shots) == 0 || len(shots[0].ImageBytes) == 0 {
		t.Fatal("no screenshot")
	}
}
