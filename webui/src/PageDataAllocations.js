import _ from "lodash";
import React from "react";
import ReactTable from "react-table";
import { Label, Input } from "reactstrap";
import { Link } from "react-router-dom";
import { ContextMenu, MenuItem, ContextMenuTrigger } from "react-contextmenu";
import AceEditor from "react-ace";
import Tabbed from "./Tabbed.js";
import { fmt_size, fmt_date_unix_ms, fmt_hex16, fmt_uptime_timeval, fmt_duration_for_display, update_query, create_query, extract_query } from "./utils.js";

import {
    DATE_FIELD,
    DATE_OR_PERCENTAGE_FIELD,
    SIZE_FIELD,
    POSITIVE_INTEGER_FIELD,
    POSITIVE_INTEGER_OR_PERCENTAGE_FIELD,
    DURATION_FIELD,
    DURATION_OR_PERCENTAGE_FIELD,
    RADIO_FIELD,
    REGEX_FIELD,

    ControlBase,
    FilterEditorBase,

    fmt_or_percent,
    backtrace_cell,
    timestamp_cell,
    identity,
    get_data_url_generic,
} from "./list-common.js";

const FIELDS = {
    from: {
        ...DATE_OR_PERCENTAGE_FIELD,
        label: "Allocated after",
        badge: value => "From " + (fmt_or_percent( fmt_date_unix_ms )( value ) || value)
    },
    to: {
        ...DATE_OR_PERCENTAGE_FIELD,
        label: "Allocated before",
        badge: value => "Until " + (fmt_or_percent( fmt_date_unix_ms )( value ) || value)
    },
    lifetime_min: {
        ...DURATION_FIELD,
        label: "From",
        badge: value => "Living at least " + fmt_duration_for_display( value )
    },
    lifetime_max: {
        ...DURATION_FIELD,
        label: "To",
        badge: value => "Living at most " + fmt_duration_for_display( value )
    },
    chain_lifetime_min: {
        ...DURATION_FIELD,
        label: "From (whole chain)",
        badge: value => "Chain living at least " + fmt_duration_for_display( value )
    },
    chain_lifetime_max: {
        ...DURATION_FIELD,
        label: "To (whole chain)",
        badge: value => "Chain living at most " + fmt_duration_for_display( value )
    },
    size_min: {
        ...SIZE_FIELD,
        label: "Min size",
        badge: value => "At least " + fmt_size( value, false ) + "B"
    },
    size_max: {
        ...SIZE_FIELD,
        label: "Max size",
        badge: value => "At most " + fmt_size( value, false ) + "B"
    },
    first_size_min: {
        ...SIZE_FIELD,
        label: "Min size (first in chain)",
        badge: value => "First at least " + fmt_size( value, false ) + "B"
    },
    first_size_max: {
        ...SIZE_FIELD,
        label: "Max size (first in chain)",
        badge: value => "First at most " + fmt_size( value, false ) + "B"
    },
    last_size_min: {
        ...SIZE_FIELD,
        label: "Min size (last in chain)",
        badge: value => "Last at least " + fmt_size( value, false ) + "B"
    },
    last_size_max: {
        ...SIZE_FIELD,
        label: "Max size (last in chain)",
        badge: value => "Last at most " + fmt_size( value, false ) + "B"
    },
    backtrace_depth_min: {
        ...POSITIVE_INTEGER_FIELD,
        label: "Min backtrace depth",
        badge: value => "Backtrace at least " + value + " deep"
    },
    backtrace_depth_max: {
        ...POSITIVE_INTEGER_FIELD,
        label: "Max backtrace depth",
        badge: value => "Backtrace at most " + value + " deep"
    },
    function_regex: {
        ...REGEX_FIELD,
        label: "Function regex",
        badge: value => "Functions matching /" + value + "/"
    },
    negative_function_regex: {
        ...REGEX_FIELD,
        label: "Negative function regex",
        badge: value => "Functions NOT matching /" + value + "/"
    },
    source_regex: {
        ...REGEX_FIELD,
        label: "Source file regex",
        badge: value => "Sources matching /" + value + "/"
    },
    negative_source_regex: {
        ...REGEX_FIELD,
        label: "Negative source file regex",
        badge: value => "Sources NOT matching /" + value + "/"
    },
    backtraces: {
        label: "Backtrace",
        badge: value => "Matching backtrace with ID " + value
    },
    deallocation_backtraces: {
        label: "Dealloc backtrace",
        badge: value => "Matching dealloc backtrace with ID " + value
    },
    group_interval_min: {
        ...DURATION_OR_PERCENTAGE_FIELD,
        label: "Min group interval",
        badge: value => "Group interval at least " + fmt_or_percent( fmt_duration_for_display )( value )
    },
    group_interval_max: {
        ...DURATION_OR_PERCENTAGE_FIELD,
        label: "Max group interval",
        badge: value => "Group interval at most " + fmt_or_percent( fmt_duration_for_display )( value )
    },
    group_max_total_usage_first_seen_min: {
        ...DURATION_OR_PERCENTAGE_FIELD,
        label: "Max group total usage first seen after",
        badge: value => "Max group total usage seen after " + fmt_or_percent( fmt_duration_for_display )( value )
    },
    group_max_total_usage_first_seen_max: {
        ...DURATION_OR_PERCENTAGE_FIELD,
        label: "Max group total usage first seen before",
        badge: value => "Max group total usage seen before " + fmt_or_percent( fmt_duration_for_display )( value )
    },
    group_allocations_min: {
        ...POSITIVE_INTEGER_FIELD,
        label: "Min allocations",
        badge: value => "At least " + value + " allocations"
    },
    group_allocations_max: {
        ...POSITIVE_INTEGER_FIELD,
        label: "Max allocations",
        badge: value => "At most " + value + " allocations"
    },
    group_leaked_allocations_min: {
        ...POSITIVE_INTEGER_OR_PERCENTAGE_FIELD,
        label: "Min leaked allocations",
        badge: value => "At least " + value + " leaked allocations"
    },
    group_leaked_allocations_max: {
        ...POSITIVE_INTEGER_OR_PERCENTAGE_FIELD,
        label: "Max leaked allocations",
        badge: value => "At most " + value + " leaked allocations"
    },
    chain_length_min: {
        ...POSITIVE_INTEGER_FIELD,
        label: "Min chain length",
        badge: value => "At least " + value + " allocations in chain"
    },
    chain_length_max: {
        ...POSITIVE_INTEGER_FIELD,
        label: "Max chain length",
        badge: value => "At most " + value + " allocations in chain"
    },
    position_in_chain_min: {
        ...POSITIVE_INTEGER_FIELD,
        label: "Min position in chain",
        badge: value => "At least " + value + " position in chain"
    },
    position_in_chain_max: {
        ...POSITIVE_INTEGER_FIELD,
        label: "Max position in chain",
        badge: value => "At most " + value + " position in chain"
    },
    lifetime: {
        ...RADIO_FIELD,
        variants: {
            "": "Show all",
            only_leaked: "Only leaked",
            only_chain_leaked: "Only chain leaked",
            only_not_deallocated_in_current_range: "Only not deallocated in current time range",
            only_deallocated_in_current_range: "Only deallocated in current time range",
            only_temporary: "Only temporary",
            only_whole_group_leaked: "Only whole group leaked"
        },
        badge: {
            only_leaked: "Only leaked",
            only_chain_leaked: "Only chain leaked",
            only_not_deallocated_in_current_range: "Only not deallocated in current range",
            only_deallocated_in_current_range: "Only deallocated in current range",
            only_temporary: "Only temporary",
            only_whole_group_leaked: "Only whole group leaked"
        }
    },
    arena: {
        ...RADIO_FIELD,
        variants: {
            "": "Show all",
            main: "Only from main arena (ptmalloc)",
            non_main: "Only from non-main arena (ptmalloc)"
        },
        badge: {
            main: "Only from main arena (ptmalloc)",
            non_main: "Only from non-main arena (ptmalloc)"
        }
    },
    mmaped: {
        ...RADIO_FIELD,
        variants: {
            "": "Show all",
            yes: "Only mmaped (ptmalloc)",
            no: "Only non-mmaped (ptmalloc)"
        },
        badge: {
            yes: "Only mmaped (ptmalloc)",
            no: "Only non-mmaped (ptmalloc)"
        }
    },
    jemalloc: {
        ...RADIO_FIELD,
        variants: {
            "": "Show all",
            yes: "Only jemalloc",
            no: "Only non-jemalloc"
        },
        badge: {
            yes: "Only jemalloc",
            no: "Only non-jemalloc"
        }
    },
    custom_filter: {
        validate: (value) => {
            if( value === "" ) {
                return false;
            }
            return true;
        },
        format: identity,
        parse: identity,
        clean: identity,
        badge: value => "Custom filter"
    }
};

const EMPTY_CUSTOM_FILTER = "return allocations();";

class FilterEditor extends FilterEditorBase {
    state = {
        customFilter: EMPTY_CUSTOM_FILTER
    }

    constructor( props ) {
        super( props, FIELDS );
    }

    render() {
        let custom_filter = this.getFieldState( "custom_filter" );
        if( custom_filter.value === "" ) {
            custom_filter.value = EMPTY_CUSTOM_FILTER;
        }

        return (
            <Tabbed>
                <div title="By timestamp" className="d-flex">
                    {this.field("from")}
                    <div className="px-2" />
                    {this.field("to")}
                </div>
                <div title="By size" className="d-flex flex-column">
                    <div className="d-flex flex-row">
                        {this.field("size_min")}
                        <div className="px-2" />
                        {this.field("size_max")}
                    </div>
                    <div className="d-flex flex-row">
                        {this.field("first_size_min")}
                        <div className="px-2" />
                        {this.field("first_size_max")}
                    </div>
                    <div className="d-flex flex-row">
                        {this.field("last_size_min")}
                        <div className="px-2" />
                        {this.field("last_size_max")}
                    </div>
                </div>
                <div title="By lifetime" className="d-flex">
                    <div className="d-flex flex-column">
                        <div className="d-flex flex-row">
                            {this.field("lifetime_min")}
                            <div className="px-2" />
                            {this.field("lifetime_max")}
                        </div>
                        <div className="d-flex flex-row">
                            {this.field("chain_lifetime_min")}
                            <div className="px-2" />
                            {this.field("chain_lifetime_max")}
                        </div>
                    </div>
                    <div className="px-2" />
                    {this.field("lifetime")}
                </div>
                <div title="By backtrace" className="d-flex flex-column">
                    <div className="d-flex flex-row">
                        {this.field("function_regex")}
                        <div className="px-2" />
                        {this.field("negative_function_regex")}
                        <div className="px-2" />
                        {this.field("source_regex")}
                        <div className="px-2" />
                        {this.field("negative_source_regex")}
                    </div>
                    <div className="d-flex flex-row">
                        {this.field("backtrace_depth_min")}
                        <div className="px-2" />
                        {this.field("backtrace_depth_max")}
                    </div>
                </div>
                <div title="By group (global)" className="d-flex flex-column">
                    <div className="d-flex flex-row">
                        {this.field("group_allocations_min")}
                        <div className="px-2" />
                        {this.field("group_allocations_max")}
                        <div className="px-2" />
                        {this.field("group_leaked_allocations_min")}
                        <div className="px-2" />
                        {this.field("group_leaked_allocations_max")}
                    </div>
                    <div className="d-flex flex-row">
                        {this.field("group_interval_min")}
                        <div className="px-2" />
                        {this.field("group_interval_max")}
                    </div>
                    <div className="d-flex flex-row">
                        {this.field("group_max_total_usage_first_seen_min")}
                        <div className="px-2" />
                        {this.field("group_max_total_usage_first_seen_max")}
                    </div>
                </div>
                <div title="By realloc." className="d-flex flex-column">
                    <div className="d-flex flex-row">
                        {this.field("chain_length_min")}
                        <div className="px-2" />
                        {this.field("chain_length_max")}
                    </div>
                    <div className="d-flex flex-row">
                        {this.field("position_in_chain_min")}
                        <div className="px-2" />
                        {this.field("position_in_chain_max")}
                    </div>
                </div>
                <div title="Misc" className="d-flex">
                    {this.field("mmaped")}
                    <div className="px-2" />
                    {this.field("arena")}
                    <div className="px-2" />
                    {this.field("jemalloc")}
                </div>
                <div title="Custom">
                    <div className="editor-pane">
                        <AceEditor
                            value={custom_filter.value}
                            name="code-editor"
                            editorProps={{ $blockScrolling: true }}
                            onChange={(code) => {
                                if( code === EMPTY_CUSTOM_FILTER ) {
                                    code = "";
                                }
                                this.onChanged( "custom_filter", code );
                            }}
                        />,
                    </div>
                </div>
            </Tabbed>
        );
    }
}

class Control extends ControlBase {
    constructor() {
        super( FIELDS, FilterEditor )
    }

    renderLeft() {
        let show_graphs = "";
        if( this.props.groupByBacktraces ) {
            show_graphs = (
                <Label check className="ml-4">
                    <Input type="checkbox" id="show-graphs" checked={this.props.showGraphs} onChange={this.onShowGraphsChanged.bind(this)} />{' '}
                    Show graphs
                </Label>
            );
        }

        return <div>
            {show_graphs}
            <Label check className="ml-5">
                <Input type="checkbox" id="show-full-backtraces" checked={this.props.showFullBacktraces} onChange={this.onShowFullBacktracesChanged.bind(this)} />{' '}
                Full backtraces
            </Label>
            <Label check className="ml-5">
                <Input type="checkbox" id="group-by-backtraces" checked={this.props.groupByBacktraces} onChange={this.onGroupByBacktracesChanged.bind(this)} />{' '}
                Group by backtraces
            </Label>
        </div>
    }

    renderMenu() {
        let fullDataUrl;
        let heaptrackUrl;
        let treeUrl;
        let flamegraphUrl;
        let scriptingUrl;
        if( this.props.dataUrl ) {
            const data_url = new URL( this.props.dataUrl );
            const q = _.omit( extract_query( data_url.search ), "count", "skip" );
            data_url.search = "?" + create_query( q ).toString();
            fullDataUrl = data_url.toString();

            data_url.pathname = "/data/" + this.props.id + "/export/heaptrack/heaptrack.dat";
            heaptrackUrl = data_url.toString();

            data_url.pathname = "/data/" + this.props.id + "/allocation_ascii_tree";
            treeUrl = data_url.toString();

            data_url.pathname = "/data/" + this.props.id + "/export/flamegraph/flame.svg";
            flamegraphUrl = data_url.toString();
        }

        return (
            <div>
                <MenuItem>
                    <a href={flamegraphUrl || "#"}>Open flamegraph</a>
                </MenuItem>
                <MenuItem>
                    <Link onClick={this.openScriptingConsole.bind( this )} onAuxClick={this.openScriptingConsole.bind( this )} to={"/console/" + this.props.id}>Open scripting console</Link>
                </MenuItem>
                <MenuItem>
                    <a href={fullDataUrl || "#"}>Download as JSON (every page)</a>
                </MenuItem>
                <MenuItem>
                    <a href={this.props.dataUrl || "#"}>Download as JSON (only this page)</a>
                </MenuItem>
                <MenuItem>
                    <a href={heaptrackUrl || "#"}>Download as Heaptrack data file</a>
                </MenuItem>
                <MenuItem>
                    <a href={treeUrl || "#"}>Download as ASCII tree</a>
                </MenuItem>
            </div>
        );
    }

    onShowGraphsChanged( event ) {
        if( this.props.onShowGraphsChanged ) {
            this.props.onShowGraphsChanged( event.target.checked )
        }
    }

    onShowFullBacktracesChanged( event ) {
        if( this.props.onShowFullBacktracesChanged ) {
            this.props.onShowFullBacktracesChanged( event.target.checked )
        }
    }

    onGroupByBacktracesChanged( event ) {
        if( this.props.onGroupByBacktracesChanged ) {
            this.props.onGroupByBacktracesChanged( event.target.checked )
        }
    }

    openScriptingConsole() {
        let code = "";
        if( this.props.filterAsScript.prologue !== "" ) {
            code += this.props.filterAsScript.prologue.trim() + "\n";
        }
        code += "let allocs = " + this.props.filterAsScript.code.trim() + ";\n";
        code += "\n";
        code += "println(\"Matched allocations: {}\", allocs.len());\n";
        code += "graph()\n";
        code += "  .add(allocs)\n";
        code += "  .save();";

        // This is a hack, but whatever.
        window.localStorage.setItem( "next-script", code );
    }
}

function get_data_url( source_url, id, params ) {
    const group_allocations = params.group_allocations === "true" || params.group_allocations === "1";
    let source;
    if( group_allocations ) {
        source = "allocation_groups";
    } else {
        source = "allocations";
    }

    return get_data_url_generic( source_url, source, id, _.omit( params, "show_full_backtraces", "group_allocations" ) )
}

export default class PageDataAllocations extends React.Component {
    state = { pages: null, data: {}, loading: false };

    componentDidUpdate( prev_props ) {
        if( this.props.location !== prev_props.location ) {
            const params = extract_query( this.props.location.search );
            this.fetchData( params );
        }
    }

    render() {
        const q = new URLSearchParams( this.props.location.search );
        const page = (parseInt( q.get( "page" ), 10 ) || 1) - 1;
        const page_size = parseInt( q.get( "page_size" ), 10 ) || 20;
        const show_graphs = q.get( "generate_graphs" ) === "true" || q.get( "generate_graphs" ) === "1";
        const show_full_backtraces = q.get( "show_full_backtraces" ) === "true" || q.get( "show_full_backtraces" ) === "1";
        const group_by_backtraces = q.get( "group_allocations" ) === "true" || q.get( "group_allocations" ) === "1";

        const columns = [
            {
                id: "timestamp",
                Header: "Timestamp",
                Cell: cell => {
                    return timestamp_cell( cell.original.timestamp, cell.original.timestamp_relative, cell.original.timestamp_relative_p );
                },
                maxWidth: 160,
                view: "allocations"
            },
            {
                id: "lifetime",
                Header: "Lifetime",
                accessor: entry => {
                    if( !entry.deallocation ) {
                        return "∞";
                    } else {
                        let interval = {
                            secs: entry.deallocation.timestamp.secs - entry.timestamp.secs,
                            fract_nsecs: entry.deallocation.timestamp.fract_nsecs - entry.timestamp.fract_nsecs,
                        };

                        if( interval.fract_nsecs < 0 ) {
                            interval.secs -= 1;
                            interval.fract_nsecs += 1000000000;
                        }

                        return fmt_uptime_timeval( interval );
                    }
                },
                maxWidth: 90,
                sortable: false,
                view: "allocations"
            },
            {
                id: "chain_lifetime",
                Header: "Lifetime (realloc.)",
                accessor: entry => {
                    if( !entry.chain_lifetime ) {
                        return "∞";
                    } else {
                        const interval = entry.chain_lifetime;
                        if( interval.fract_nsecs < 0 ) {
                            interval.secs -= 1;
                            interval.fract_nsecs += 1000000000;
                        }

                        return fmt_uptime_timeval( interval );
                    }
                },
                maxWidth: 120,
                sortable: false,
                view: "allocations"
            },
            {
                Header: "Address",
                id: "address",
                accessor: "address_s",
                maxWidth: 140,
                view: "allocations"
            },
            {
                Header: "Thread",
                Cell: cell => {
                    return fmt_hex16( cell.value );
                },
                accessor: "thread",
                maxWidth: 60,
                sortable: false,
                view: "allocations"
            },
            {
                Header: "Size",
                Cell: cell => {
                    return <div>{fmt_size( cell.value )}<br />({fmt_size( cell.value + cell.original.extra_space )})</div>;
                },
                accessor: "size",
                maxWidth: 85,
                view: "allocations"
            },
            {
                Header: "Realloc.",
                Cell: cell => {
                    if( cell.original.chain_length === 1 ) {
                        return "";
                    } else {
                        let value = "";
                        switch( cell.original.position_in_chain ) {
                            case 0:
                                value = "1st";
                                break;
                            case 1:
                                value = "2nd";
                                break;
                            case 3:
                                value = "3rd";
                                break;
                            default:
                                value = cell.original.position_in_chain + "th";
                        }

                        return <div>{value}<br />({cell.original.chain_length} total)</div>;
                    }
                },
                view: "allocations",
                maxWidth: 70
            },
            {
                Header: "Allocator",
                Cell: cell => {
                    if( cell.original.is_jemalloc ) {
                        return "jemalloc";
                    } else {
                        if( cell.original.is_mmaped ) {
                            return <div>system<br />(mmapped)</div>;
                        } else {
                            if( cell.original.in_main_arena ) {
                                return "system";
                            } else {
                                return <div>system<br />(other arena)</div>;
                            }
                        }
                    }
                },
                maxWidth: 90,
                sortable: false,
                view: "allocations"
            },
            {
                id: "all.min_timestamp",
                Header: <div>(global)<br />First allocation</div>,
                Cell: cell => {
                    return timestamp_cell( cell.original.all.min_timestamp, cell.original.all.min_timestamp_relative, cell.original.all.min_timestamp_relative_p );
                },
                maxWidth: 160,
                view: "grouped"
            },
            {
                id: "all.max_timestamp",
                Header: <div>(global)<br />Last allocation</div>,
                Cell: cell => {
                    return timestamp_cell( cell.original.all.max_timestamp, cell.original.all.max_timestamp_relative, cell.original.all.max_timestamp_relative_p );
                },
                maxWidth: 160,
                view: "grouped"
            },
            {
                id: "all.max_total_usage_first_seen_at",
                Header: <div>(global)<br />Max total usage at</div>,
                Cell: cell => {
                    return timestamp_cell( cell.original.all.max_total_usage_first_seen_at, cell.original.all.max_total_usage_first_seen_at_relative, cell.original.all.max_total_usage_first_seen_at_relative_p );
                },
                maxWidth: 160,
                view: "grouped"
            },
            {
                id: "all.interval",
                Header: <div>(global)<br />Interval</div>,
                Cell: cell => {
                    return fmt_uptime_timeval( cell.original.all.interval );
                },
                maxWidth: 95,
                view: "grouped"
            },
            {
                id: "all.allocated_count",
                Header: <div>(global)<br />Allocated</div>,
                Cell: cell => {
                    return cell.original.all.allocated_count;
                },
                maxWidth: 70,
                view: "grouped"
            },
            {
                id: "all.leaked_count",
                Header: <div>(global)<br />Leaked</div>,
                Cell: cell => {
                    return cell.original.all.leaked_count;
                },
                maxWidth: 75,
                view: "grouped"
            },

            {
                id: "only_matched.min_timestamp",
                Header: <div>(matched)<br />First allocation</div>,
                Cell: cell => {
                    return timestamp_cell( cell.original.only_matched.min_timestamp, cell.original.only_matched.min_timestamp_relative, cell.original.only_matched.min_timestamp_relative_p );
                },
                maxWidth: 160,
                view: "grouped"
            },
            {
                id: "only_matched.max_timestamp",
                Header: <div>(matched)<br />Last allocation</div>,
                Cell: cell => {
                    return timestamp_cell( cell.original.only_matched.max_timestamp, cell.original.only_matched.max_timestamp_relative, cell.original.only_matched.max_timestamp_relative_p );
                },
                maxWidth: 160,
                view: "grouped"
            },
            {
                id: "only_matched.interval",
                Header: <div>(matched)<br />Interval</div>,
                Cell: cell => {
                    return fmt_uptime_timeval( cell.original.only_matched.interval );
                },
                maxWidth: 95,
                view: "grouped"
            },
            {
                id: "only_matched.allocated_count",
                Header: <div>(matched)<br />Allocated</div>,
                Cell: cell => {
                    return cell.original.only_matched.allocated_count;
                },
                maxWidth: 75,
                view: "grouped"
            },
            {
                id: "only_matched.leaked_count",
                Header: <div>(matched)<br />Leaked</div>,
                Cell: cell => {
                    return cell.original.only_matched.leaked_count;
                },
                maxWidth: 75,
                view: "grouped"
            },
            {
                id: "only_matched.size",
                Header: <div>(matched)<br />Size</div>,
                Cell: cell => {
                    return fmt_size( cell.original.only_matched.size );
                },
                maxWidth: 85,
                view: "grouped"
            }
        ].filter( (column) => {
            if( column.view === "allocations" && this.state.group ) {
                return false;
            }

            if( column.view === "grouped" && !this.state.group ) {
                return false;
            }

            return true;
        });

        const get_sorted = (sorted) => {
            let sort_by = null;
            let order = null;
            if( sorted.length > 0 ) {
                sort_by = sorted[ 0 ].id;
                order = sorted[ 0 ].desc ? "dsc" : "asc";
            }

            return { sort_by, order };
        };

        const table_sorted = [];
        const p = extract_query( this.props.location.search );
        if( p.sort_by || p.order ) {
            table_sorted[ 0 ] = {
                id: p.sort_by || "timestamp",
                desc: (p.order || "asc") === "dsc"
            };
        }

        let expanded = {};
        for( let i = 0; i < page_size; ++i ) {
            expanded[ i ] = true;
        }

        return (
            <div className="PageDataAllocations">
                <Control
                    id={this.props.id}
                    location={this.props.location}
                    totalCount={this.state.data.total_count}
                    page={page}
                    pageSize={page_size}
                    showGraphs={show_graphs}
                    showFullBacktraces={show_full_backtraces}
                    groupByBacktraces={group_by_backtraces}
                    filter={extract_query( this.props.location.search )}
                    filterAsScript={this.state.filterAsScript}
                    dataUrl={this.state.lastDataUrl}
                    onPageChange={(page) => update_query( this.props, {page: page + 1} )}
                    onShowGraphsChanged={value => {
                        update_query( this.props, {generate_graphs: value} );
                    }}
                    onShowFullBacktracesChanged={value => {
                        update_query( this.props, {show_full_backtraces: value} );
                    }}
                    onGroupByBacktracesChanged={value => {
                        const location = update_query( this.props, {
                            group_allocations: value,
                            sort_by: this.state.otherSortBy,
                            order: this.state.otherOrder
                        });
                        this.setState({
                            otherSortBy: p.sort_by,
                            otherOrder: p.order
                        });
                    }}
                    onFilterChange={(filter) => {
                        update_query( this.props, filter );
                    }}
                />
                <ReactTable
                    manual
                    data={this.state.data.allocations}
                    pages={this.state.pages}
                    loading={this.state.loading}
                    columns={columns}
                    defaultSorted={table_sorted}
                    page={page}
                    pageSize={page_size}
                    onFetchData={(state) => {
                        const params = extract_query( this.props.location.search );
                        params.page = state.page;
                        params.page_size = state.pageSize;
                        this.fetchData( {...params, ...get_sorted( state.sorted )} );
                    }}
                    onSortedChange={(sorted) => {
                        const s = get_sorted( sorted );
                        update_query( this.props, s );
                    }}
                    onPageSizeChange={(page_size, page) => update_query( this.props, {page: page + 1, page_size} )}
                    showPaginationBottom={false}
                    SubComponent={row => {
                        const q = _.omit( extract_query( this.props.location.search ), "count", "skip", "group_allocations", "sort_by", "order" );

                        const allocation_backtrace =
                            <div className="backtrace-cell" onContextMenu={event => {
                                    const lq = _.cloneDeep(q);
                                    lq.backtraces = row.original.backtrace_id;
                                    const url = "/#" + this.props.location.pathname + "?" + create_query( lq ).toString();

                                    this.setState({
                                        showOnlyAllocationsUrl: url,
                                        selectedBacktrace: row.original.backtrace_id
                                    });
                                    return this.allocation_menu_trigger.handleContextClick( event );
                                }}>{backtrace_cell( show_full_backtraces, row.original.backtrace )}
                            </div>;

                        const deallocation_backtrace = (row.original.deallocation && row.original.deallocation.backtrace) ?
                            <div className="backtrace-cell" onContextMenu={event => {
                                    const lq = _.cloneDeep(q);
                                    lq.deallocation_backtraces = row.original.deallocation.backtrace_id;
                                    const url = "/#" + this.props.location.pathname + "?" + create_query( lq ).toString();

                                    this.setState({
                                        showOnlyAllocationsUrl: url,
                                        selectedBacktrace: row.original.deallocation.backtrace_id
                                    });
                                    return this.deallocation_menu_trigger.handleContextClick( event );
                                }}>{backtrace_cell( show_full_backtraces, row.original.deallocation.backtrace )}
                            </div>
                            : null;

                        const chain_deallocation_backtrace = (row.original.chain_deallocation && row.original.chain_deallocation.backtrace) ?
                            <div className="backtrace-cell" onContextMenu={event => {
                                    const lq = _.cloneDeep(q);
                                    lq.deallocation_backtraces = row.original.chain_deallocation.backtrace_id;
                                    const url = "/#" + this.props.location.pathname + "?" + create_query( lq ).toString();

                                    this.setState({
                                        showOnlyAllocationsUrl: url,
                                        selectedBacktrace: row.original.chain_deallocation.backtrace_id
                                    });
                                    return this.deallocation_menu_trigger.handleContextClick( event );
                                }}>{backtrace_cell( show_full_backtraces, row.original.chain_deallocation.backtrace )}
                            </div>
                            : null;

                        const s = {fontStyle: "italic", color: "black"};
                        let cell;
                        if (deallocation_backtrace && chain_deallocation_backtrace) {
                            cell = [
                                <div style={s}>Allocated at:</div>,
                                allocation_backtrace,
                                <div style={{marginTop: "1rem"}}></div>,
                                <div style={s}>Reallocated at:</div>,
                                deallocation_backtrace,
                                <div style={{marginTop: "1rem"}}></div>,
                                <div style={s}>Deallocated at:</div>,
                                chain_deallocation_backtrace
                            ]
                        } else if (deallocation_backtrace) {
                            cell = [
                                <div style={s}>Allocated at:</div>,
                                allocation_backtrace,
                                <div style={{marginTop: "1rem"}}></div>,
                                <div style={s}>Deallocated at:</div>,
                                deallocation_backtrace
                            ];
                        } else {
                            cell = allocation_backtrace;
                        }

                        let graph = "";
                        if( row.original.only_matched && row.original.only_matched.graph_url ) {
                            const url_preview = (this.props.sourceUrl || "") + row.original.only_matched.graph_preview_url;
                            const url_full = (this.props.sourceUrl || "") + row.original.only_matched.graph_url;
                            graph = (
                                <a href={url_full} target="_blank">
                                    <img src={url_preview} style={{maxHeight: "15rem"}} />
                                </a>
                            );
                        }

                        return <div className="backtrace-parent">
                            <div>
                                {cell}
                            </div>
                            {graph}
                        </div>;
                    }}
                    expanded={expanded}
                />
                <ContextMenuTrigger id="allocation_context_menu" ref={c => this.allocation_menu_trigger = c}>
                    <div />
                </ContextMenuTrigger>
                <ContextMenuTrigger id="deallocation_context_menu" ref={c => this.deallocation_menu_trigger = c}>
                    <div />
                </ContextMenuTrigger>
                <ContextMenu id="allocation_context_menu">
                    <MenuItem>
                        <a href={this.state.showOnlyAllocationsUrl || "#"}>Show only allocations with this backtrace...</a>
                    </MenuItem>
                </ContextMenu>
                <ContextMenu id="deallocation_context_menu">
                    <MenuItem>
                        <a href={this.state.showOnlyAllocationsUrl || "#"}>Show only allocations with this deallocation backtrace...</a>
                    </MenuItem>
                </ContextMenu>
            </div>
        );
    }

    fetchData( params ) {
        if( this.state.loading ) {
            return;
        }

        const data_url = get_data_url( this.props.sourceUrl, this.props.id, params );
        if( this.state.lastDataUrl === data_url ) {
            return;
        }

        this.setState({
            loading: true,
            filterAsScript: null
        });
        fetch( data_url )
            .then( response => {
                if( response.status !== 200 ) {
                    return response.text().then( error => Promise.reject( error ) );
                }

                return response.json();
             })
            .then( data => {
                if( data.error ) {
                    return Promise.reject( data.error );
                }

                const pages = Math.floor( (data.total_count / params.page_size) ) + (((data.total_count % params.page_size) !== 0) ? 1 : 0);
                this.setState({
                    data,
                    pages,
                    loading: false,
                    lastDataUrl: data_url,
                    group: params.group_allocations === "true" || params.group_allocations === "1"
                });
            })
            .catch( error => {
                // TODO: Put the error message on the page itself.
                alert( "Failed to fatch data: " + error );
                this.setState({
                    data: {},
                    pages: null,
                    loading: false,
                    lastDataUrl: data_url,
                    group: false
                });
            });

        const url = (this.props.sourceUrl || "") + "/data/" + this.props.id + "/allocation_filter_to_script?" + create_query( params ).toString();
        fetch( url, {
            cache: "no-cache"
        })
            .then( response => response.json() )
            .then( response => {
                this.setState({
                    filterAsScript: response
                });
            })
            .catch( error => {
                this.setState({
                    filterAsScript: null
                });
            });
    }
}
