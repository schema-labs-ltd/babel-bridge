package main

import (
	"context"
	"fmt"
	"slices"

	"go.mau.fi/whatsmeow/types"
)

// SetPresence marks the account online while an app is in front of the user, as WhatsApp Web
// does. WhatsApp only sends typing indicators to an online device, and an online device also
// takes the phone's notifications, so the server sets it back to unavailable when no app is.
// The wish is kept, so it is sent again after a reconnect or once the push name is known.
func (c *Client) SetPresence(ctx context.Context, available bool) error {
	c.presenceMu.Lock()
	defer c.presenceMu.Unlock()
	c.wantsPresence = available
	return c.sendPresenceLocked(ctx)
}

// sendPresenceLocked sends the wanted presence; the caller holds presenceMu.
func (c *Client) sendPresenceLocked(ctx context.Context) error {
	if c.client == nil || !c.client.IsConnected() {
		return fmt.Errorf("not connected")
	}
	state := types.PresenceUnavailable
	if c.wantsPresence {
		state = types.PresenceAvailable
	}
	return c.client.SendPresence(ctx, state)
}

// resendPresence restores an online state WhatsApp dropped, such as after a reconnect. It
// reads the wish under the same lock as SetPresence, so it can't undo a later "unavailable".
func (c *Client) resendPresence() {
	go func() {
		c.presenceMu.Lock()
		defer c.presenceMu.Unlock()
		if !c.wantsPresence {
			return
		}
		if err := c.sendPresenceLocked(c.ctx); err != nil {
			SendEvent(NewLogEvent("warn", fmt.Sprintf("presence not restored: %v", err)))
		}
	}()
}

// phoneAddress turns a hidden (LID) user address into the phone address the app uses for
// contacts and senders, when WhatsApp has told this device the mapping.
// presenceChatIDs lists the addresses a typing indicator's chat can be stored under: its
// phone address first, as live messages use, then its LID address, as history sync can keep.
func (c *Client) presenceChatIDs(source types.MessageSource) []string {
	var alternate types.JID
	if !source.IsGroup {
		// In a private chat the sender is the chat, so its phone address is the chat's too.
		alternate = source.SenderAlt
	}
	phone := c.phoneAddress(normalizeAddress(source.Chat, alternate))
	ids := []string{phone.String()}
	if hidden := c.hiddenAddress(phone); hidden != phone {
		ids = append(ids, hidden.String())
	}
	if raw := normalizeAddress(source.Chat, types.JID{}); raw != phone && !slices.Contains(ids, raw.String()) {
		ids = append(ids, raw.String())
	}
	return ids
}

// hiddenAddress maps a phone address to its LID address when the store knows it.
func (c *Client) hiddenAddress(jid types.JID) types.JID {
	if jid.Server != types.DefaultUserServer || c.client == nil || c.client.Store == nil || c.client.Store.LIDs == nil {
		return jid
	}
	if lid, err := c.client.Store.LIDs.GetLIDForPN(c.ctx, jid); err == nil && lid.User != "" {
		return lid.ToNonAD()
	}
	return jid
}

func (c *Client) phoneAddress(jid types.JID) types.JID {
	if jid.Server != types.HiddenUserServer || c.client == nil || c.client.Store == nil || c.client.Store.LIDs == nil {
		return jid
	}
	if phone, err := c.client.Store.LIDs.GetPNForLID(c.ctx, jid); err == nil && phone.User != "" {
		return phone.ToNonAD()
	}
	return jid
}
