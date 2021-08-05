import _ from "lodash";
import React from "react";
import AceEditor from "react-ace";
import { Button } from "reactstrap";
import { Link } from "react-router-dom";
import Feather from "./Feather.js";

export default class PageDataConsole extends React.Component {
    state = {
        code: "graph()\n  .add(\"Leaked\", allocations().only_leaked())\n  .add(\"All\", allocations())\n  .save();",
        running: false,
        response: null
    }

    constructor() {
        super()
    }

    componentDidMount() {
        let script = window.localStorage.getItem( "next-script" );
        window.localStorage.removeItem( "next-script" );

        if( script === null ) {
            script = window.localStorage.getItem( "current-script" );
        }

        if( script !== null ) {
            this.setState({
                code: script
            });
        }
    }

    render() {
        let output = "";
        if( this.state.response !== null ) {
            const list = [];
            let counter = 0;
            for( const entry of (this.state.response.output || []) ) {
                counter += 1;
                if( entry.kind === "println" ) {
                    const key = "print-" + counter;
                    list.push(
                        <div key={key} className="println">
                            {entry.value}
                        </div>
                    );
                } else if( entry.kind === "image" ) {
                    const url = (this.props.sourceUrl || "") + entry.url;
                    const key = "file-" + entry.checksum;
                    list.push(
                        <div key={key} className="script-file">
                            <div>
                                {entry.path}
                            </div>
                            <a href={url} target="_blank">
                                <img src={url} />
                            </a>
                        </div>
                    );
                }
            }

            if( this.state.response.status == "error" ) {
                list.push(
                    <div key="response-error" className="error">ERROR: {this.state.response.message}</div>
                );
            } else if( this.state.response.elapsed ) {
                list.push(
                    <div key="response-message" className="message">Script finished in {this.state.response.elapsed}s</div>
                );
            }

            output = (
                <div>
                    <h1 className="h3">Output</h1>
                    <br />
                    <div className="output-body">
                        {list}
                    </div>
                    <br />
                </div>
            );
        }
        return (
            <div className="PageDataConsole">
                <div className="navbar flex-column flex-md-nonwrap shadow w-100 px-3 py-2">
                    <div className="d-flex justify-content-between w-100">
                        <div className="d-flex align-items-center flex-grow-0">
                            <Link to="/" className="mr-3"><Feather name="grid" /></Link>
                            <Link to={"/overview/" + this.props.id} className="mr-3"><Feather name="bar-chart-2" /></Link>
                            <Link to={this.props.location} className="mr-3"><Feather name="anchor" /></Link>
                        </div>
                        <div className="flex-grow-1 text-center">
                            Scripting console
                        </div>
                    </div>
                </div>
                <div className="main">
                    <div className="editor-pane">
                        <AceEditor
                            value={this.state.code}
                            name="code-editor"
                            editorProps={{ $blockScrolling: true }}
                            onChange={(code) => {
                                window.localStorage.setItem("current-script", code);
                                this.setState({
                                    code
                                });
                            }}
                        />,
                        <br />
                        <div className="d-flex justify-content-between">
                            <Button outline color="primary" className="ml-2 btn-sm" onClick={this.run.bind( this )} style={{minWidth: "8em"}}>{
                                (!this.state.running) ? ("Run") : ("Running...")
                            }</Button>
                            <Button outline color="dark" className="ml-2 btn-sm" onClick={this.copy.bind( this )}>Copy script to clipboard</Button>
                        </div>
                    </div>
                    {output}
                </div>
            </div>
        );
    }

    copy() {
        navigator.clipboard.writeText(this.state.code);
    }

    run() {
        this.setState({
            running: true
        });

        fetch( (this.props.sourceUrl || "") + "/data/" + this.props.id + "/execute_script", {
            method: "POST",
            cache: "no-cache",
            body: this.state.code
        })
        .then( response => response.json() )
        .then( response => {
            this.setState({
                running: false,
                response
            });
        })
        .catch( error => {
            this.setState({
                running: false,
                response: {
                    error: "Failed: " + error
                }
            });
        });
    }
}
