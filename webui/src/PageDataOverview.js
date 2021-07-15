import _ from "lodash";
import React from "react";
import Graph from "./Graph.js";
import { Button, ButtonGroup, DropdownMenu, DropdownItem } from "reactstrap";
import { ContextMenu, MenuItem, ContextMenuTrigger } from "react-contextmenu";
import { Link } from "react-router-dom";
import classNames from "classnames";
import { fmt_date_unix, fmt_uptime, fmt_size } from "./utils.js";
import Feather from "./Feather.js";

class Switcher extends React.Component {
    state = { selected: null, x0: null, x1: null }

    render() {
        let children = this.props.children;
        if( !_.isArray( children ) ) {
            children = [children];
        }

        const selected = this.state.selected || (children[0] || {}).key;

        const buttons = children.map( (child) => {
            const out =
                <Button key={child.key} outline color="primary" active={selected === child.key} onClick={() => this.setState( {selected: child.key} )}>
                    {child.props.title}
                </Button>;
            return out;
        });

        const inner = React.Children.map( this.props.children, (child) => {
            const className = classNames(
                child.props.className,
                {"d-none": selected !== child.key}
            );

            return React.cloneElement( child, {className, title: null} );
        });

        return (
            <div className="Switcher">
                <div className="d-flex justify-content-center">
                    <ButtonGroup>
                        {buttons}
                    </ButtonGroup>
                </div>
                {inner}
            </div>
        );
    }
}

export default class PageDataOverview extends React.Component {
    state = {}

    componentDidMount() {
        fetch( this.props.sourceUrl + "/list" )
            .then( response => response.json() )
            .then( list => this.setState( {general: _.find( list, entry => entry.id === this.props.id ) } ) );

        fetch( (this.props.sourceUrl || "") + "/data/" + this.props.id + "/timeline" )
            .then( rsp => rsp.json() )
            .then( json => this.setState( {timeline: json} ) );

        fetch( (this.props.sourceUrl || "") + "/data/" + this.props.id + "/fragmentation_timeline" )
            .then( rsp => rsp.json() )
            .then( json => this.setState( {fragmentation_timeline: json} ) );
    }

    render() {
        let general = null;
        let inner = [];

        if( this.state.general ) {
            general = (
                <div>
                    <table id="overview-table">
                        <tbody>
                            <tr>
                                <td>ID</td>
                                <td>{this.props.id}</td>
                            </tr>
                            <tr>
                                <td>Executable</td>
                                <td>{this.state.general.executable}</td>
                            </tr>
                            <tr>
                                <td>Architecture</td>
                                <td>{this.state.general.architecture}</td>
                            </tr>
                            <tr>
                                <td>Total runtime</td>
                                <td>{fmt_uptime( this.state.general.runtime.secs )}</td>
                            </tr>
                            <tr>
                                <td>Unique backtraces</td>
                                <td>{this.state.general.unique_backtrace_count}</td>
                            </tr>
                            <tr>
                                <td>Max. backtrace depth</td>
                                <td>{this.state.general.maximum_backtrace_depth}</td>
                            </tr>
                        </tbody>
                    </table>
                </div>
            );
        }

        if( this.state.timeline ) {
            inner.push(
                <Switcher key="s1">
                    <Graph
                        key="memory"
                        title="Memory usage"
                        data={this.state.timeline}
                        y_accessor="allocated_size"
                        y_label=""
                        onZoom={this.onZoom.bind(this)}
                        onRightClick={this.onRightClick.bind(this)}
                        x0={this.state.x0}
                        x1={this.state.x1}
                        fill={true}
                        xUnit="unix_timestamp"
                    />
                    <Graph
                        key="size_delta"
                        title="Memory usage delta"
                        data={this.state.timeline}
                        y_accessor="size_delta"
                        y_label=""
                        onZoom={this.onZoom.bind(this)}
                        onRightClick={this.onRightClick.bind(this)}
                        x0={this.state.x0}
                        x1={this.state.x1}
                        fill={true}
                        xUnit="unix_timestamp"
                    />
                    <Graph
                        key="count"
                        title="Live allocations"
                        data={this.state.timeline}
                        y_accessor="allocated_count"
                        y_label=""
                        onZoom={this.onZoom.bind(this)}
                        onRightClick={this.onRightClick.bind(this)}
                        x0={this.state.x0}
                        x1={this.state.x1}
                        fill={true}
                        xUnit="unix_timestamp"
                    />
                    <Graph
                        key="count_delta"
                        title="Live allocations delta"
                        data={this.state.timeline}
                        y_accessor="count_delta"
                        y_label=""
                        onZoom={this.onZoom.bind(this)}
                        onRightClick={this.onRightClick.bind(this)}
                        x0={this.state.x0}
                        x1={this.state.x1}
                        fill={true}
                        xUnit="unix_timestamp"
                    />
                    <Graph
                        key="allocations"
                        title="Alloc/s"
                        data={this.state.timeline}
                        y_accessor="allocations"
                        y_label=""
                        onZoom={this.onZoom.bind(this)}
                        onRightClick={this.onRightClick.bind(this)}
                        x0={this.state.x0}
                        x1={this.state.x1}
                        fill={true}
                        xUnit="unix_timestamp"
                    />
                    <Graph
                        key="deallocations"
                        title="Dealloc/s"
                        data={this.state.timeline}
                        y_accessor="deallocations"
                        y_label=""
                        onZoom={this.onZoom.bind(this)}
                        onRightClick={this.onRightClick.bind(this)}
                        x0={this.state.x0}
                        x1={this.state.x1}
                        fill={true}
                        xUnit="unix_timestamp"
                    />
                </Switcher>
            );
        }

        if( this.state.timeline ) {
            inner.push(
                <Switcher key="s3">
                    <Graph
                        key="leaked_size"
                        title="Leaked memory"
                        data={this.state.timeline}
                        y_accessor="leaked_size"
                        y_label=""
                        onZoom={this.onZoom.bind(this)}
                        onRightClick={this.onRightClick.bind(this)}
                        x0={this.state.x0}
                        x1={this.state.x1}
                        fill={true}
                        xUnit="unix_timestamp"
                    />
                    <Graph
                        key="leaked_count"
                        title="Leaked allocations"
                        data={this.state.timeline}
                        y_accessor="leaked_count"
                        y_label=""
                        onZoom={this.onZoom.bind(this)}
                        onRightClick={this.onRightClick.bind(this)}
                        x0={this.state.x0}
                        x1={this.state.x1}
                        fill={true}
                        xUnit="unix_timestamp"
                    />
                </Switcher>
            );
        }

        if( this.state.fragmentation_timeline ) {
            inner.push(
                <Switcher key="s2">
                    <Graph
                        key="fragmentation"
                        title="Memory lost to fragmentation"
                        data={this.state.fragmentation_timeline}
                        y_accessor="fragmentation"
                        y_label=""
                        onZoom={this.onZoom.bind(this)}
                        onRightClick={this.onRightClick.bind(this)}
                        x0={this.state.x0}
                        x1={this.state.x1}
                        fill={true}
                        xUnit="unix_timestamp"
                    />
                </Switcher>
            );
        }

        const prefix = (this.props.sourceUrl || "") + "/data/" + this.props.id;

        return (
            <div className="PageDataOverview">
                <div className="navbar flex-column flex-md-nonwrap shadow w-100 px-3 py-2">
                    <div className="d-flex justify-content-between w-100">
                        <div className="d-flex align-items-center flex-grow-0">
                            <Link to="/" className="mr-3"><Feather name="grid" /></Link>
                            <Link to={this.props.location} className="mr-3"><Feather name="anchor" /></Link>
                        </div>
                        <div className="flex-grow-1 text-center">
                            Overview of {this.props.id}
                        </div>
                    </div>
                </div>
                <div className="pt-4 px-4">
                    <div className="d-flex justify-content-between flex-wrap" style={{gap: "1rem"}}>
                        {general}
                        <div id="subpage-list">
                            <div>List of allocations</div>
                            <div style={{marginLeft: "1rem"}}>
                                <div><Link to={"/allocations/" + this.props.id}>Everything</Link></div>
                                <div>
                                    <Link to={"/allocations/" + this.props.id + "?lifetime=only_leaked"}>Only leaked</Link>
                                    &nbsp;(<a href={prefix + "/export/flamegraph/flame.svg?lifetime=only_leaked"}>flamegraph</a>)
                                </div>
                            </div>
                            <div>Download</div>
                            <div style={{marginLeft: "1rem"}}>
                                <div><a href={(this.props.sourceUrl || "") + "/data/" + this.props.id + "/export/heaptrack/heaptrack.dat"}>...as Heaptrack data</a></div>
                            </div>
                            <div><Link to={"/address_space/" + this.props.id + "?lifetime=only_not_deallocated_in_current_range&mmaped=no"}>Address space fragmentation</Link></div>
                            <div>
                                <a href={(this.props.sourceUrl || "") + "/data/" + this.props.id + "/dynamic_constants_ascii_tree/dynamic_constants_" + this.props.id + ".txt"}>Dynamically allocated constants</a>
                                &nbsp;(<a href={(this.props.sourceUrl || "") + "/data/" + this.props.id + "/dynamic_constants/dynamic_constants_" + this.props.id + ".json"}>.json</a>)
                            </div>
                            <div>
                                <a href={(this.props.sourceUrl || "") + "/data/" + this.props.id + "/dynamic_statics_ascii_tree/dynamic_statics_" + this.props.id + ".txt"}>Dynamically allocated statics</a>
                                &nbsp;(<a href={(this.props.sourceUrl || "") + "/data/" + this.props.id + "/dynamic_statics/dynamic_statics_" + this.props.id + ".json"}>.json</a>)
                            </div>
                        </div>
                    </div>
                    <br />
                    <br />
                    {inner}
                    <ContextMenuTrigger id="overview_context_menu" ref={c => this.context_trigger = c}></ContextMenuTrigger>
                    <ContextMenu id="overview_context_menu">
                        <MenuItem>
                            <Link to={this.allocationsLink()}>Allocations at {this.getSelectedDate()}</Link>
                        </MenuItem>
                        <MenuItem>
                            <Link to={this.allocationsRangeLink()}>Allocations in {this.getSelectedRange()}</Link>
                        </MenuItem>
                    </ContextMenu>
                </div>
            </div>
        );
    }

    allocationsLink() {
        if( this.state.context_x === undefined ) {
            return "/";
        }

        const x = Math.floor( this.state.context_x );
        return "/allocations/" + this.props.id + "?from=" + x + "&to=" + x;
    }

    allocationsRangeLink() {
        if( this.state.context_range === undefined ) {
            return "/";
        }

        const x0 = Math.floor( this.state.context_range[ 0 ] );
        const x1 = Math.floor( this.state.context_range[ 1 ] );
        return "/allocations/" + this.props.id + "?from=" + x0 + "&to=" + x1;
    }

    getSelectedDate() {
        if( !this.state.timeline || this.state.context_x === undefined ) {
            return "";
        }

        return fmt_date_unix( this.state.context_x );
    }

    getSelectedRange() {
        if( !this.state.timeline || this.state.context_range === undefined ) {
            return "";
        }

        const r = this.state.context_range;
        return fmt_date_unix( r[0] ) + " - " + fmt_date_unix( r[1] );
    }

    onZoom( min, max ) {
        this.setState( {x0: min, x1: max} );
    }

    onRightClick( {event, x, x0, x1} ) {
        this.setState( {context_x: x, context_range: [x0, x1]} );
        this.context_trigger.handleContextClick( event );
    }
}
