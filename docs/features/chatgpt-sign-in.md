# Sign in with ChatGPT

An alternative to `OPENAI_API_KEY` for text AI. See the [README](../../README.md) for the other environment variables.

## How it works

Instead of an API key, you can sign in with your ChatGPT account so translation, language detection, AI compose, styled replies, style analysis and topics count toward your ChatGPT plan rather than API billing. This uses OpenAI's [Sign in with ChatGPT](https://developers.openai.com/siwc/token-sharing-open-source) flow for open-source, self-hosted apps: the server registers its own OAuth client with PKCE, so no client secret or API key is needed.

- **Native apps (iOS, iPadOS, macOS):** tap **Sign in with ChatGPT**, approve in the sheet, and the app passes the result to your server.
- **Web:** click **Sign in with ChatGPT**, approve in the new tab, then copy the `http://127.0.0.1…` address that tab ends on (it won't load) and paste it into **Finish sign-in**. ChatGPT only redirects to `127.0.0.1`, which is your own computer rather than a hosted server.

While signed in, the server uses ChatGPT for every text AI request and doesn't fall back to the API key. Choose the model from the ones your account lists; the GPT model setting applies only to API-key requests. Usage limits show as errors rather than silently switching billing. Sign out to go back to the API key.

ChatGPT plan usage covers the Responses API only, so voice note transcription and speech still need `OPENAI_API_KEY`. Tokens are stored in `chatgpt-auth.json` in `WA_DATA_DIR` with owner-only permissions; keep that directory private. OpenAI offers this to open-source and personal self-hosted apps; a paid or hosted service built on this code needs to join OpenAI's [interest form](https://openai.com/form/sign-in-with-chatgpt-interest/) first.
