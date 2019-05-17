import React from "react";
import FEATHER_SPRITE_URL from "feather-icons/dist/feather-sprite.svg";

export default class Feather extends React.Component {
    render() {
        const src = FEATHER_SPRITE_URL + "#" + this.props.name;
        return (
            <svg className="feather" onClick={this.props.onClick}>
                <use xlinkHref={src} />
            </svg>
        );
    }
}
