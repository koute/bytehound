import React from "react";
import { Link } from "react-router-dom";
import Feather from "./Feather.js";
import { timestamp_cell, backtrace_cell } from "./list-common.js";
import { fmt_size, fmt_hex16, fmt_uptime_timeval } from "./utils.js";

function size_cell( size, last_size ) {
    let classes = [];
    if( size !== last_size ) {
        classes.push( "value-updated" );
    }

    if( size == 0 ) {
        return <div className={classes}>
            <div className="d-flex flex-column align-items-end">
                0
            </div>
        </div>;
    };

    return (
        <div className={classes}>
            <div className="d-flex flex-column align-items-end">
                <div>{fmt_size( size )}</div>
                <div>({size})</div>
            </div>
        </div>
    );
}

export default class PageDataMapDetails extends React.Component {
    state = {}

    componentDidMount() {
        fetch( (this.props.sourceUrl || "") + "/data/" + this.props.id + "/maps?with_regions=true&with_usage_history=true&id=" + this.props.map_id )
            .then( rsp => rsp.json() )
            .then( json => this.setState( {details: json.maps[0]} ) );
    }

    render() {
        let contents = null;

        if( this.state.details ) {
            const region_rows = [];
            for( let i = 0; i < this.state.details.regions.length; ++i ) {
                const region = this.state.details.regions[ i ];
                let lifetime = "∞";

                if( region.deallocation ) {
                    let interval = {
                        secs: region.deallocation.timestamp.secs - region.timestamp.secs,
                        fract_nsecs: region.deallocation.timestamp.fract_nsecs - region.timestamp.fract_nsecs,
                    };

                    if( interval.fract_nsecs < 0 ) {
                        interval.secs -= 1;
                        interval.fract_nsecs += 1000000000;
                    }

                    lifetime = fmt_uptime_timeval( interval );
                }

                let perms = "";
                if( region.is_readable ) {
                    perms += "r";
                } else {
                    perms += "-";
                }

                if( region.is_writable ) {
                    perms += "w";
                } else {
                    perms += "-";
                }

                if( region.is_executable ) {
                    perms += "x";
                } else {
                    perms += "-";
                }

                if( region.is_shared ) {
                    perms += "s";
                } else {
                    perms += "p";
                }

                const source_timestamp = timestamp_cell( region.timestamp, region.timestamp_relative, region.timestamp_relative_p );

                region_rows.push(
                    <tr>
                        <td>
                            {source_timestamp}
                        </td>
                        <td>
                            {lifetime}
                        </td>
                        <td>
                            {region.address_s}
                        </td>
                        <td>
                            {perms}
                        </td>
                        <td>
                            <div>{fmt_size( region.size )}<br />({region.size / 4096} pages)</div>
                        </td>
                        <td>
                            {region.name}
                        </td>
                    </tr>
                );
            }

            const usage_history_rows = [];
            let last_usage = {};
            for( let i = 0; i < this.state.details.usage_history.length; ++i ) {
                const usage = this.state.details.usage_history[ i ];
                usage_history_rows.push(
                    <tr>
                        <td>{timestamp_cell( usage.timestamp, usage.timestamp_relative, usage.timestamp_relative_p )}</td>
                        <td>{size_cell( usage.address_space, last_usage.address_space )}</td>
                        <td>{size_cell( usage.rss, last_usage.rss )}</td>
                    </tr>
                );

                last_usage = usage;
            }

            let backtrace = null;
            if( this.state.details.source ) {
                backtrace = (
                    <div>
                        <div className="mt-4"></div>
                        <h5>Backtrace</h5>
                        <div className="backtrace-cell">{backtrace_cell( true, this.state.details.source.backtrace )}</div>
                    </div>
                );
            }

            contents = (
                <div>
                    <h5>Regions</h5>
                    <table>
                        <tr>
                            <th>Timestamp</th>
                            <th>Lifetime</th>
                            <th>Address</th>
                            <th>Permissions</th>
                            <th>Size</th>
                            <th>Name</th>
                        </tr>
                        {region_rows}
                    </table>

                    <div className="mt-4"></div>

                    <h5>Usage history</h5>
                    <table>
                        <tr>
                            <th>Timestamp</th>
                            <th>Address space</th>
                            <th>RSS</th>
                        </tr>
                        {usage_history_rows}
                    </table>

                    {backtrace}
                </div>
            );
        }

        const prefix = (this.props.sourceUrl || "") + "/data/" + this.props.id;

        return (
            <div className="PageDataMapDetails">
                <div className="navbar flex-column flex-md-nonwrap shadow w-100 px-3 py-2">
                    <div className="d-flex justify-content-between w-100">
                        <div className="d-flex align-items-center flex-grow-0">
                            <Link to="/" className="mr-3"><Feather name="grid" /></Link>
                            <Link to={this.props.location} className="mr-3"><Feather name="anchor" /></Link>
                        </div>
                        <div className="flex-grow-1 text-center">
                            Map details
                        </div>
                    </div>
                </div>

                <div className="pt-4 px-4">
                    {contents}
                </div>
            </div>
        );
    }
}
