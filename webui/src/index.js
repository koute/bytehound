import "./import-bootstrap.js";
import "bootswatch/dist/spacelab/bootstrap.min.css";
import "font-awesome/css/font-awesome.min.css";
import "react-table/react-table.css";

import React from "react";
import ReactDOM from "react-dom";
import { HashRouter, withRouter } from "react-router-dom";

import App from "./App";

import "./index.css";

const AppWithRouter = withRouter(App);

let sourceUrl;

// Is there a better way to detect that we're running under Parcel?
if( module.hot ) {
    sourceUrl = "http://localhost:8080";
} else {
    sourceUrl = window.location.origin;
}

ReactDOM.render(
    <HashRouter>
        <AppWithRouter sourceUrl={sourceUrl} />
    </HashRouter>,
    document.getElementById( "root" )
);

if( module.hot ) {
    module.hot.accept();
}
