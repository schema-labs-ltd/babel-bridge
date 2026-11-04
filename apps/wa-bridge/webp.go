package main

// isWebP reports whether data is a RIFF WebP file, the format WhatsApp stickers use.
func isWebP(data []byte) bool {
	return len(data) >= 12 && string(data[0:4]) == "RIFF" && string(data[8:12]) == "WEBP"
}

// isAnimatedWebP reports whether a WebP file has the animation flag in its VP8X header.
func isAnimatedWebP(data []byte) bool {
	return isWebP(data) && len(data) >= 21 && string(data[12:16]) == "VP8X" && data[20]&0x02 != 0
}

// webPSize reads a WebP file's canvas size, which WhatsApp clients use to lay out a sticker
// before it downloads. It returns zero for a size it can't read.
func webPSize(data []byte) (uint32, uint32) {
	if !isWebP(data) || len(data) < 30 {
		return 0, 0
	}
	le24 := func(b []byte) uint32 { return uint32(b[0]) | uint32(b[1])<<8 | uint32(b[2])<<16 }
	switch string(data[12:16]) {
	case "VP8X":
		return le24(data[24:27]) + 1, le24(data[27:30]) + 1
	case "VP8L":
		if data[20] != 0x2f {
			return 0, 0
		}
		bits := uint32(data[21]) | uint32(data[22])<<8 | uint32(data[23])<<16 | uint32(data[24])<<24
		return bits&0x3fff + 1, (bits>>14)&0x3fff + 1
	case "VP8 ":
		return uint32(data[26]) | uint32(data[27]&0x3f)<<8, uint32(data[28]) | uint32(data[29]&0x3f)<<8
	}
	return 0, 0
}
