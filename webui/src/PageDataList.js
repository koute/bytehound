import React from "react";
import ReactTable from "react-table";
import { Button } from "reactstrap";
import { Link } from "react-router-dom";
import { fmt_uptime, fmt_size, fmt_date_unix } from "./utils.js";

export default class PageDataList extends React.Component {
    state = { datasets: [] }

    componentDidMount() {
        this.updateDatasetList();
    }

    render() {
        const columns = [
            {
                id: "timestamp",
                Header: "Timestamp",
                Cell: cell => {
                    return fmt_date_unix( cell.original.timestamp.secs );
                },
                maxWidth: 200,
                accessor: d => d.timestamp,
                sortMethod: (a, b) => {
                    if (a.secs === b.secs) {
                        return a.fract_nsecs > b.fract_nsecs ? 1 : -1;
                    }
                    return a.secs > b.secs ? 1 : -1;
                }
            },
            {
                Header: "Binary",
                accessor: "executable"
            },
            {
                id: "runtime",
                Header: "Runtime",
                Cell: cell => {
                    return fmt_uptime( cell.original.runtime.secs );
                },
                maxWidth: 150
            },
            {
                Header: "Allocated",
                Cell: cell => {
                    return fmt_size( cell.value ) + "B";
                },
                accessor: "final_allocated",
                maxWidth: 150
            },
            {
                Header: "Allocated count",
                Cell: cell => {
                    return fmt_size( cell.value );
                },
                accessor: "final_allocated_count",
                maxWidth: 150
            },
            {
                Header: "Architecture",
                accessor: "architecture",
                maxWidth: 150
            },
            {
                Header: "...",
                Cell: row => {
                    return (
                        <Link to={"/overview/" + row.original.id}>Open</Link>
                    );
                },
                maxWidth: 150
            }
        ];

        const data = this.state.datasets;
        return (
            <div className="PageDataList">
                <div className="navbar shadow w-100 px-3 py-2">
                    <div className="w-100 text-center">
                        Currently loaded data
                    </div>
                </div>
                <div className="px-4 pt-4">
                    <ReactTable
                        columns={columns}
                        data={this.state.datasets}
                        resolveData={data => this.preprocess( data )}
                        defaultSorted={[{id: "timestamp", desc: true}]}
                    />
                </div>
            </div>
        );
    }

    preprocess( data ) {
        return data.map( in_row => {
            let row = {...in_row};
            row.executable = row.executable.match( /[^/]+$/ )[ 0 ]
            return row;
        });
    }

    updateDatasetList() {
        fetch( this.props.sourceUrl + "/list" )
            .then( response => response.json() )
            .then( list => this.setState( { datasets: list } ) );
    }
}
