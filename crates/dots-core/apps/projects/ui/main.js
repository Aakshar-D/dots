// Projects' page: the screen in `#app`, talking to Dots through the client Dots puts in the page.
import { mount } from "./projects.js";

mount(document.getElementById("app"), window.dots);
