package main

import (
	"bytes"
	"io"
	"regexp"
	"strings"
	"sync"
)

var (
	jidPattern   = regexp.MustCompile(`(?i)\b[0-9A-Za-z._-]{1,128}@(?:s\.whatsapp\.net|c\.us|g\.us|lid|broadcast|newsletter|hosted)\b`)
	phonePattern = regexp.MustCompile(`\b\d{8,15}\b`)
)

// redactLog removes chat ids, phone numbers, and protocol XML from a log line.
// Byte counts such as "10485760 bytes" stay so media diagnostics remain useful.
func redactLog(input string) string {
	withoutProtocol := stripProtocolNodes(input)
	withoutJIDs := jidPattern.ReplaceAllString(withoutProtocol, "[chat]")
	return phonePattern.ReplaceAllStringFunc(withoutJIDs, func(match string) string {
		index := strings.Index(withoutJIDs, match)
		if index >= 0 && isSizeCount(withoutJIDs, index, index+len(match)) {
			return match
		}
		return "[redacted]"
	})
}

func stripProtocolNodes(input string) string {
	start := strings.Index(input, "<")
	end := strings.LastIndex(input, ">")
	if start < 0 || end <= start {
		return input
	}
	return input[:start] + "[protocol]" + input[end+1:]
}

func isSizeCount(text string, start, end int) bool {
	after := strings.TrimLeft(text[end:], " \t")
	if strings.HasPrefix(after, "bytes") || strings.HasPrefix(after, "byte") {
		return true
	}
	before := strings.TrimRight(text[:start], " \t")
	return strings.HasSuffix(before, "size:") || strings.HasSuffix(before, "size")
}

// redactingWriter filters complete lines before they reach stderr.
type redactingWriter struct {
	mu  sync.Mutex
	buf bytes.Buffer
	out io.Writer
}

func (w *redactingWriter) Write(p []byte) (int, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	if _, err := w.buf.Write(p); err != nil {
		return 0, err
	}
	for {
		data := w.buf.Bytes()
		newline := bytes.IndexByte(data, '\n')
		if newline < 0 {
			break
		}
		line := append([]byte(nil), data[:newline+1]...)
		w.buf.Next(newline + 1)
		if _, err := io.WriteString(w.out, redactLog(string(line))); err != nil {
			return 0, err
		}
	}
	return len(p), nil
}
