// Facilities for creating and updating open posts

import PostView, {OPView} from '../view'
import {Post, PostData, OP, ThreadData, TextState} from '../models'
import {SpliceResponse} from '../../client'
import {applyMixins, makeFrag, setAttrs} from '../../util'
import {posts, isMobile} from '../../state'
import {parseTerminatedLine} from '../render/body'
import {read, write} from '../../render'
import {posts as lang, ui} from '../../lang'
import {send, message} from "../../connection"

// Current PostForm and model instances
export let postForm: FormView
export let postModel: Post & FormModel

// Form Model of an OP post
export class OPFormModel extends OP implements FormModel {
	bodyLength: number
	parsedLines: number
	view: FormView

	commitChar: (char: string) => void
	commitBackspace: () => void
	lastBodyLine: () => string
	parseInput: (val: string) => void

	constructor(id: number) {

		// TODO: Persist id to state.mine

		const oldModel = posts.get(id) as OP,
			oldView = oldModel.view
		oldView.unbind()

		// Copy the parent model's state and data
		super(extractAttrs(oldModel) as ThreadData)

		// Replace old model and view pair with the postForm pair
		posts.addOP(this)
		postForm = new OPFormView(this)
		postModel = this
		oldView.el.replaceWith(postForm.el)

		this.bodyLength = this.parsedLines = 0

		// TODO: Hide [Reply] button

	}
}

// Override mixin for post authoring models
class FormModel {
	bodyLength: number  // Compound length of the input text body
	parsedLines: number // Number of closed, commited and parsed lines
	body: string
	view: PostView & FormView
	state: TextState

	spliceLine: (line: string, msg: SpliceResponse) => string

	// Append a character to the model's body and reparse the line, if it's a
	// newline
	append(code: number) {
		const char = String.fromCharCode(code)
		if (char === "\n") {
			this.view.terminateLine(this.parsedLines++)
		}
		this.body += char
	}

	// Remove the last character from the model's body
	backspace() {
		const {state} = this
		this.body = this.body.slice(0, -1)
	}

	// Splice the last line of the body
	splice(msg: SpliceResponse) {
		this.spliceLine(this.lastBodyLine(), msg)
	}

	// Compare new value to old and generate apropriate commands
	parseInput(val: string): void {
		const old = this.state.line,
			lenDiff = val.length - old.length,
			exceeding = this.bodyLength + lenDiff - 2000

		// If exceeding max body lenght, shorten the value, trim $input and try
		// again
		if (exceeding > 0) {
			this.view.trimInput(exceeding)
			return this.parseInput(val.slice(0, -exceeding))
		}

		if (lenDiff === 1 && val.slice(0, -1) === old) {
			return this.commitChar(val.slice(-1))
		}
		if (lenDiff === -1 && old.slice(0, -1) === val) {
			return this.commitBackspace()
		}
	}

	// Commit a character appendage to the end of the line to the server
	commitChar(char: string) {
		this.state.line += char
		this.bodyLength++
		send(message.append, char.charCodeAt(0))
	}

	// Send a message about removing the last character of the line to the
	// server
	commitBackspace() {
		this.state.line = this.state.line.slice(0, -1)
		this.bodyLength--
		send(message.backspace, null)
	}

	// Return the last line of the body
	lastBodyLine(): string {
		const lines = this.body.split("\n")
		return lines[lines.length - 1]
	}
}

applyMixins(OPFormModel, FormModel)

// Post creation and update view
class FormView extends PostView {
	model: Post & FormModel
	$input: HTMLTextAreaElement
	$sizer: HTMLPreElement // Used for dynamically resizing $input
	$done: HTMLInputElement
	$postControls: Element

	constructor(model: Post) {
		super(model)
		this.renderInputs()
	}

	// Render extra input fields for inputting text and optionally uploading
	// images
	renderInputs() {
		this.$input = document.createElement("textarea")
		const attrs: StringMap = {
			id: "text-input",
			name: "body",
			rows: "1",
		}
		if (isMobile) {
			attrs["autocomplete"] = ""
		}
		setAttrs(this.$input, attrs)
		this.$input.addEventListener("input", (event: Event) =>
			this.model.parseInput((event.target as Element).value))

		this.$sizer = document.createElement("pre")

		this.$postControls = document.createElement("div")
		this.$postControls.id = "post-controls"

		this.$done = document.createElement("input")
		setAttrs(this.$done, {
			name: "done",
			type: "button",
			value: ui.done,
		})
		this.$postControls.append(this.$done)

		write(() => {
			this.$blockquote.innerHTML = ""
			this.$blockquote.append(this.$input)
			this.el.append(this.$postControls)
			this.$input.focus()
			document.body.append(this.$sizer)
			this.resizeInput("")
		})

		// TODO: UploadForm integrations

	}

	// Resize $input according to the text inside. Can't really use async
	// methods of render.ts here. Need an immediate response.
	resizeInput(val = this.$input.value) {
		this.$sizer.textContent = val
		const min =
			300
			+ this.$input.getBoundingClientRect().left
			+ this.el.getBoundingClientRect().left
			+ document.body.scrollLeft * 2
		const size = Math.max(this.$sizer.offsetWidth + 20, min)
		this.$input.style.width = size + "px"
	}

	// Trim $input from the end by the suplied length
	trimInput(length: number) {
		this.$input.value = this.$input.value.slice(0, -length)
	}

	// Parse and replace the temporary like closed by $input with a proper
	// parsed line
	terminateLine(num: number) {
		const html = parseTerminatedLine(this.model.lastBodyLine(), this.model),
			frag = makeFrag(html)
		read(() => {
			const el = this.$blockquote.querySelector(`span:nth-child(${num})`)
			write(() =>
				el.replaceWith(frag))
		})
	}
}

// FormView of an OP post
class OPFormView extends FormView implements OPView {
	$omit: Element
	model: OP & FormModel
	renderOmit: () => void

	constructor(model: OP) {
		super(model)
	}
}

applyMixins(OPFormView, OPView)

// Extract all non-function attributes from a model
function extractAttrs(src: {[key: string]: any}): {[key: string]: any} {
	const attrs: {[key: string]: any} = {}
	for (let key in src) {
		if (typeof src[key] !== "function") {
			attrs[key] = src[key]
		}
	}
	return attrs
}
