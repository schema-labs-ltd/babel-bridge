package main

import (
	"slices"
	"testing"

	"go.mau.fi/whatsmeow/types"
)

func TestTypingReachesTheChatUnderEveryAddress(t *testing.T) {
	c := &Client{}
	lid := types.NewJID("123456789", types.HiddenUserServer)
	phone := types.NewJID("447700900001", types.DefaultUserServer)
	private := types.MessageSource{Chat: lid, Sender: lid, SenderAlt: phone}
	if got := c.presenceChatIDs(private); !slices.Equal(got, []string{phone.String(), lid.String()}) {
		t.Fatalf("private chat addresses = %v", got)
	}
	group := types.NewJID("120363000000000001", types.GroupServer)
	if got := c.presenceChatIDs(types.MessageSource{Chat: group, Sender: lid, SenderAlt: phone, IsGroup: true}); !slices.Equal(got, []string{group.String()}) {
		t.Fatalf("group addresses = %v", got)
	}
	if got := c.presenceChatIDs(types.MessageSource{Chat: phone, Sender: phone}); !slices.Equal(got, []string{phone.String()}) {
		t.Fatalf("phone chat addresses = %v", got)
	}
}
