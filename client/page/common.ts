import { fetchBoard, fetchThread } from "../fetch"
import { PageState, posts, setBoardConfig } from '../state'
import renderThread from './thread'
import { renderFresh as renderBoard } from './board'
import { makeFrag } from "../util"
import { setExpandAll } from "../posts/images"

// Load a page (either board or thread) and render it once the ready promise
// has been resolved
export default async function (
	{board, thread, lastN}: PageState,
	ready: Promise<void>
) {
	const [html, err] = thread
		? await fetchThread(board, thread, lastN)
		: await fetchBoard(board)
	if (err) {
		throw err
	}

	await ready

	posts.clear()
	const frag = makeFrag(html)
	extractConfigs(frag)
	setExpandAll(false)

	if (thread) {
		renderThread(frag)
	} else {
		renderBoard(frag)
	}
}

// Find board configurations in the HTML and apply them
export function extractConfigs(ns: NodeSelector) {
	const conf = ns.querySelector("#board-configs").textContent
	setBoardConfig(JSON.parse(conf))
}
