import { useState } from 'react'

declare global {
  interface Window {
    api: { save(name: string): Promise<string> }
  }
}

function save(name: string) {
  return window.api.save(name)
}

function App() {
    const [name, setName] = useState("");

    return (
        <div className="container">
            <input
                value={name}
                onChange={e => setName(e.target.value)}
            />

            <button onClick={() => save(name)}>
                Save
            </button>
        </div>
    );
}

export default App
