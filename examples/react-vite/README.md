# Electron + React + Vite example

An ordinary Electron application in the layout of the `electron-vite-react`
template: the main process in `electron/main.ts`, a preload script in
`electron/preload.ts` and a React renderer in `src/`: a text input bound to
React state and a button whose click saves the name through IPC in the main
process. CI builds it to check that NativeBoat works end to end.

Nothing in it knows about NativeBoat. With Electron installed it runs as usual
(`npm install`, then `npm run dev`).

## Building a native executable

Install the runtime dependencies (React) and build:

```powershell
npm ci --omit=dev
nativeboat build
```

`dist\Notes.exe` is the whole application: a native Win32 window with a
native text box and button, no Electron, Chromium or WebView. The saved name
goes to `%APPDATA%\Notes\name.txt`.
